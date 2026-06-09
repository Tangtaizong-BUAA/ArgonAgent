//! Runtime-owned tool contract mediation for native DeepSeek/Qwen loops.
//!
//! DEPRECATED-COMPAT: Phase 2 TCML callers should import contract mediation
//! from `crate::tcml`. This file still hosts the implementation while schema
//! validation and repair internals migrate behind the TCML facade.
//!
//! Provider parsers may produce native tool calls, DSML/XML fallbacks, or plain
//! text candidates. This module is the single boundary that resolves aliases,
//! validates required arguments, applies low-risk repairs, and converts model
//! mistakes into model-readable tool errors instead of RuntimeError panics.

use crate::tcml::{
    apply_low_risk_repairs, build_tool_manifest,
    extract_content_tool_call_candidates as tcml_extract_content_tool_call_candidates,
    model_readable_tool_error, parse_tool_arguments, run_tool_manifest_doctor,
    validate_required_arguments, AliasRegistry, ParsedToolArguments, ParsedToolCall, ToolErrorCode,
};
use crate::tool_execution::{ToolExecutionArgs, ToolExecutionResult};
use researchcode_kernel::tool::find_tool_spec;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolMediationStatus {
    Ready,
    Repaired,
    Rejected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolMediationEvent {
    pub event_type: String,
    pub payload_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolInputRepair {
    pub tool_name: String,
    pub issue_path: String,
    pub repair_rule: String,
    pub before_summary: String,
    pub after_summary: String,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelReadableToolError {
    pub error_code: String,
    pub tool_name: String,
    pub short_message: String,
    pub field_errors: Vec<String>,
    pub retryable: bool,
    pub retry_hint: Option<String>,
    pub retry_example: Option<String>,
    pub counts_against_budget: bool,
    pub suggested_replacement: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediatedToolCall {
    pub provider_tool_call_id: Option<String>,
    pub requested_tool_id: String,
    pub tool_id: String,
    pub arguments_json: String,
    pub arguments: ParsedToolArguments,
    pub execution_args: ToolExecutionArgs,
    pub status: ToolMediationStatus,
    pub events: Vec<ToolMediationEvent>,
    pub repairs: Vec<ToolInputRepair>,
    pub error: Option<ModelReadableToolError>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamingToolCallAccumulator {
    buffers: BTreeMap<String, String>,
}

impl StreamingToolCallAccumulator {
    pub fn push_delta(&mut self, call_key: &str, delta: &str) -> ToolMediationEvent {
        self.buffers
            .entry(call_key.to_string())
            .or_default()
            .push_str(delta);
        ToolMediationEvent {
            event_type: "tool_call.delta_received".to_string(),
            payload_json: format!(
                "{{\"call_key\":{},\"delta_bytes\":{},\"assembled_bytes\":{}}}",
                json_string(call_key),
                delta.len(),
                self.buffers
                    .get(call_key)
                    .map(|value| value.len())
                    .unwrap_or_default()
            ),
        }
    }

    pub fn complete(&mut self, call_key: &str) -> (String, ToolMediationEvent) {
        let assembled = self.buffers.remove(call_key).unwrap_or_default();
        let event = ToolMediationEvent {
            event_type: "tool_call.assembly_completed".to_string(),
            payload_json: format!(
                "{{\"call_key\":{},\"assembled_bytes\":{}}}",
                json_string(call_key),
                assembled.len()
            ),
        };
        (assembled, event)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCallLedger {
    proposed: BTreeSet<String>,
    completed: BTreeSet<String>,
    duplicate_results: BTreeSet<String>,
}

impl ToolCallLedger {
    pub fn propose(&mut self, tool_call_id: &str) {
        self.proposed.insert(tool_call_id.to_string());
    }

    pub fn record_result(&mut self, tool_call_id: &str) -> bool {
        if !self.completed.insert(tool_call_id.to_string()) {
            self.duplicate_results.insert(tool_call_id.to_string());
            return false;
        }
        true
    }

    pub fn missing_results(&self) -> Vec<String> {
        self.proposed
            .difference(&self.completed)
            .cloned()
            .collect::<Vec<_>>()
    }

    pub fn duplicate_results(&self) -> Vec<String> {
        self.duplicate_results.iter().cloned().collect::<Vec<_>>()
    }

    pub fn exactly_once_ok(&self) -> bool {
        self.missing_results().is_empty() && self.duplicate_results.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolNameResolution {
    pub requested_tool_id: String,
    pub canonical_tool_id: String,
    pub alias_applied: bool,
    pub suggested_replacement: Option<String>,
}

pub fn resolve_tool_name(requested_tool_id: &str) -> ToolNameResolution {
    let requested = requested_tool_id.trim();
    let resolution = AliasRegistry::resolve(requested);
    if find_tool_spec(&resolution.canonical_tool_id).is_some() {
        return ToolNameResolution {
            requested_tool_id: resolution.requested_tool_id,
            canonical_tool_id: resolution.canonical_tool_id,
            alias_applied: resolution.alias_applied,
            suggested_replacement: resolution.suggested_replacement,
        };
    }
    ToolNameResolution {
        requested_tool_id: requested.to_string(),
        canonical_tool_id: requested.to_string(),
        alias_applied: false,
        suggested_replacement: suggest_tool_replacement(requested),
    }
}

pub fn mediate_tool_call(requested_tool_id: &str, arguments_json: &str) -> MediatedToolCall {
    mediate_tool_call_with_provider_id(requested_tool_id, None::<&str>, arguments_json)
}

fn unknown_tool_error(
    requested_tool_id: &str,
    suggested_replacement: Option<String>,
) -> ModelReadableToolError {
    let mut error = model_readable_tool_error(
        ToolErrorCode::UnknownTool,
        requested_tool_id.to_string(),
        format!(
            "Unknown tool '{}'. Use the stable ResearchCode tool manifest.",
            requested_tool_id
        ),
        true,
        suggested_replacement,
    );
    error.retry_hint = Some(
        "Pick one canonical tool_id from the current manifest and resend the call.".to_string(),
    );
    error.retry_example =
        Some(r#"{"tool_id":"file.read","arguments":{"path":"README.md"}}"#.to_string());
    error
}

fn malformed_json_error(tool_id: &str) -> ModelReadableToolError {
    let mut error = model_readable_tool_error(
        ToolErrorCode::MalformedToolJson,
        tool_id.to_string(),
        "Tool arguments must be a complete JSON object.",
        true,
        Some(tool_id.to_string()),
    );
    error.retry_hint =
        Some("Resend the same tool with a complete JSON object for arguments.".to_string());
    error.retry_example = Some(format!(
        "{{\"tool_id\":{},\"arguments\":{{}}}}",
        json_string(tool_id)
    ));
    error
}

fn schema_validation_failed_error(
    tool_id: &str,
    validation_issues: &[String],
) -> ModelReadableToolError {
    let mut error = model_readable_tool_error(
        ToolErrorCode::SchemaValidationFailed,
        tool_id.to_string(),
        format!(
            "Tool arguments failed validation: {}",
            validation_issues.join(", ")
        ),
        true,
        Some(tool_id.to_string()),
    );
    error.field_errors = validation_issues.to_vec();
    error.retry_hint =
        Some("Fill every required field and keep side-effect payloads exact.".to_string());
    error.retry_example = Some(schema_retry_example(tool_id));
    error
}

pub fn mediate_tool_call_with_provider_id(
    requested_tool_id: &str,
    provider_tool_call_id: Option<&str>,
    arguments_json: &str,
) -> MediatedToolCall {
    let mut events = Vec::new();
    let mut repairs = Vec::new();
    let provider_fragment = provider_tool_call_id
        .map(|id| format!(",\"provider_tool_call_id\":{}", json_string(id)))
        .unwrap_or_default();
    events.push(ToolMediationEvent {
        event_type: "tool.name.resolution_started".to_string(),
        payload_json: format!(
            "{{\"requested_tool\":{},\"argument_bytes\":{}{}}}",
            json_string(requested_tool_id),
            arguments_json.len(),
            provider_fragment
        ),
    });
    let resolution = resolve_tool_name(requested_tool_id);
    if resolution.alias_applied {
        events.push(ToolMediationEvent {
            event_type: "tool.name.alias_resolved".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"resolved_tool\":{}}}",
                json_string(&resolution.requested_tool_id),
                json_string(&resolution.canonical_tool_id)
            ),
        });
    }
    let Some(spec) = find_tool_spec(&resolution.canonical_tool_id) else {
        let doctor_report = run_tool_manifest_doctor();
        let error = unknown_tool_error(
            &resolution.requested_tool_id,
            resolution.suggested_replacement.clone(),
        );
        events.push(ToolMediationEvent {
            event_type: "tool.name.unknown".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"suggested_replacement\":{},\"available_tools\":{}}}",
                json_string(&resolution.requested_tool_id),
                json_optional_string(resolution.suggested_replacement.as_deref()),
                json_array_strings(&build_tool_manifest().canonical_tool_ids)
            ),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.doctor.snapshot".to_string(),
            payload_json: format!(
                "{{\"ok\":{},\"manifest_hash\":{},\"checked_tools\":{},\"failure_count\":{}}}",
                doctor_report.ok,
                json_string(&doctor_report.manifest_hash),
                doctor_report.checked_tools,
                doctor_report.failures.len()
            ),
        });
        if !doctor_report.ok {
            events.push(ToolMediationEvent {
                event_type: "tool.doctor.failed".to_string(),
                payload_json: format!(
                    "{{\"manifest_hash\":{},\"failures\":{}}}",
                    json_string(&doctor_report.manifest_hash),
                    json_array_strings(&doctor_report.failures)
                ),
            });
        }
        events.push(ToolMediationEvent {
            event_type: "tool.error.model_readable".to_string(),
            payload_json: error.to_payload_json(),
        });
        events.push(ToolMediationEvent {
            event_type: "model.retry_requested".to_string(),
            payload_json: format!(
                "{{\"reason\":\"unknown_tool\",\"requested_tool\":{}}}",
                json_string(&resolution.requested_tool_id)
            ),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.call.rejected".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"resolved_tool\":{},\"reason\":\"unknown_tool\"}}",
                json_string(&resolution.requested_tool_id),
                json_string(&resolution.canonical_tool_id)
            ),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.mediation.completed".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"resolved_tool\":{},\"status\":\"rejected\",\"reason\":\"unknown_tool\"}}",
                json_string(&resolution.requested_tool_id),
                json_string(&resolution.canonical_tool_id)
            ),
        });
        let continuation_tool_id = error
            .suggested_replacement
            .as_deref()
            .filter(|tool_id| find_tool_spec(tool_id).is_some())
            .unwrap_or("file.read")
            .to_string();
        return rejected_call(
            requested_tool_id,
            &continuation_tool_id,
            provider_tool_call_id,
            arguments_json,
            events,
            error,
        );
    };

    if !tool_arguments_json_is_object(arguments_json) {
        let error = malformed_json_error(&spec.tool_id);
        events.push(ToolMediationEvent {
            event_type: "tool.validation_failed".to_string(),
            payload_json: format!(
                "{{\"tool_id\":{},\"issues\":[\"malformed JSON object\"],\"retryable\":true}}",
                json_string(&spec.tool_id)
            ),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.error.model_readable".to_string(),
            payload_json: error.to_payload_json(),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.mediation.completed".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"resolved_tool\":{},\"status\":\"rejected\",\"reason\":\"malformed_tool_json\"}}",
                json_string(&resolution.requested_tool_id),
                json_string(&spec.tool_id)
            ),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.call.rejected".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"resolved_tool\":{},\"reason\":\"malformed_tool_json\"}}",
                json_string(&resolution.requested_tool_id),
                json_string(&spec.tool_id)
            ),
        });
        return rejected_call(
            requested_tool_id,
            &spec.tool_id,
            provider_tool_call_id,
            arguments_json,
            events,
            error,
        );
    }

    events.push(ToolMediationEvent {
        event_type: "tool.validation_started".to_string(),
        payload_json: format!(
            "{{\"tool_id\":{},\"argument_bytes\":{}}}",
            json_string(&spec.tool_id),
            arguments_json.len()
        ),
    });
    let mut arguments = parse_tool_arguments(arguments_json);
    normalize_alias_arguments_for_tool(&spec.tool_id, &mut arguments);
    apply_low_risk_repairs(
        &spec,
        arguments_json,
        &mut arguments,
        &mut events,
        &mut repairs,
    );
    let validation_issues = validate_required_arguments(&spec, &arguments);
    if !validation_issues.is_empty() {
        let error = schema_validation_failed_error(&spec.tool_id, &validation_issues);
        events.push(ToolMediationEvent {
            event_type: "tool.validation_failed".to_string(),
            payload_json: format!(
                "{{\"tool_id\":{},\"issues\":{},\"retryable\":true}}",
                json_string(&spec.tool_id),
                json_array_strings(&validation_issues)
            ),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.error.model_readable".to_string(),
            payload_json: error.to_payload_json(),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.call.rejected".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"resolved_tool\":{},\"reason\":\"schema_validation_failed\"}}",
                json_string(&resolution.requested_tool_id),
                json_string(&spec.tool_id)
            ),
        });
        events.push(ToolMediationEvent {
            event_type: "tool.mediation.completed".to_string(),
            payload_json: format!(
                "{{\"requested_tool\":{},\"resolved_tool\":{},\"status\":\"rejected\",\"reason\":\"schema_validation_failed\"}}",
                json_string(&resolution.requested_tool_id),
                json_string(&spec.tool_id)
            ),
        });
        return rejected_call(
            requested_tool_id,
            &spec.tool_id,
            provider_tool_call_id,
            arguments_json,
            events,
            error,
        );
    }
    events.push(ToolMediationEvent {
        event_type: "tool.validation_passed".to_string(),
        payload_json: format!(
            "{{\"tool_id\":{},\"repair_count\":{}}}",
            json_string(&spec.tool_id),
            repairs.len()
        ),
    });
    events.push(ToolMediationEvent {
        event_type: "tool.mediation.completed".to_string(),
        payload_json: format!(
            "{{\"requested_tool\":{},\"resolved_tool\":{},\"status\":{},\"repair_count\":{},\"permission_required\":{},\"concurrency_safe\":{}}}",
            json_string(&resolution.requested_tool_id),
            json_string(&spec.tool_id),
            json_string(if repairs.is_empty() { "ready" } else { "repaired" }),
            repairs.len(),
            spec.permission_required,
            spec.concurrency_safe
        ),
    });
    push_pipeline_completed(
        &mut events,
        &resolution.requested_tool_id,
        &spec.tool_id,
        if repairs.is_empty() {
            "ready"
        } else {
            "repaired"
        },
        None,
    );
    let execution_args = tool_args(&arguments);
    MediatedToolCall {
        provider_tool_call_id: provider_tool_call_id.map(|id| id.to_string()),
        requested_tool_id: requested_tool_id.to_string(),
        tool_id: spec.tool_id,
        arguments_json: arguments_to_json(&arguments, arguments_json),
        arguments,
        execution_args,
        status: if repairs.is_empty() {
            ToolMediationStatus::Ready
        } else {
            ToolMediationStatus::Repaired
        },
        events,
        repairs,
        error: None,
    }
}

pub fn model_error_to_tool_result(
    tool_call_id: &str,
    requested_tool_id: &str,
    error: &ModelReadableToolError,
) -> ToolExecutionResult {
    ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_id: requested_tool_id.to_string(),
        ok: false,
        preview: format!(
            "{} retryable={}",
            error.error_code.to_ascii_lowercase(),
            error.retryable
        ),
        detail_json: error.to_tool_result_detail_json(),
        exit_code: None,
    }
}

pub fn extract_content_tool_call_candidates(raw: &str) -> Vec<ParsedToolCall> {
    tcml_extract_content_tool_call_candidates(raw)
}

impl ModelReadableToolError {
    pub fn to_payload_json(&self) -> String {
        format!(
            "{{\"error_code\":{},\"tool_name\":{},\"short_message\":{},\"field_errors\":{},\"retryable\":{},\"retry_hint\":{},\"retry_example\":{},\"counts_against_budget\":{},\"suggested_replacement\":{}}}",
            json_string(&self.error_code),
            json_string(&self.tool_name),
            json_string(&self.short_message),
            json_array_strings(&self.field_errors),
            self.retryable,
            json_optional_string(self.retry_hint.as_deref()),
            json_optional_string(self.retry_example.as_deref()),
            self.counts_against_budget,
            json_optional_string(self.suggested_replacement.as_deref())
        )
    }

    pub fn to_tool_result_detail_json(&self) -> String {
        format!(
            "{{\"ok\":false,\"error_code\":{},\"tool_name\":{},\"message\":{},\"field_errors\":{},\"recoverable\":{},\"retry_hint\":{},\"retry_example\":{},\"counts_against_budget\":{},\"suggested_replacement\":{},\"next_action_hint\":{}}}",
            json_string(&self.error_code),
            json_string(&self.tool_name),
            json_string(&self.short_message),
            json_array_strings(&self.field_errors),
            self.retryable,
            json_optional_string(self.retry_hint.as_deref()),
            json_optional_string(self.retry_example.as_deref()),
            self.counts_against_budget,
            json_optional_string(self.suggested_replacement.as_deref()),
            json_string(
                self.retry_hint
                    .as_deref()
                    .unwrap_or("Use only tools from the current ResearchCode manifest and resend exactly one corrected tool call."),
            )
        )
    }
}

fn schema_retry_example(tool_id: &str) -> String {
    match tool_id {
        "file.read" => r#"{"path":"README.md","offset":0,"limit":120}"#.to_string(),
        "file.write" => r#"{"path":"relative/path.txt","content":"exact new file content"}"#.to_string(),
        "file.edit" => {
            r#"{"path":"relative/path.txt","old_string":"exact old text","new_string":"exact new text"}"#.to_string()
        }
        "shell.command" => r#"{"command":"cargo test -p researchcode-runtime --lib"}"#.to_string(),
        "task.dispatch" => r#"{"prompt":"Inspect README and summarize findings"}"#.to_string(),
        _ => "{}".to_string(),
    }
}

fn rejected_call(
    requested_tool_id: &str,
    tool_id: &str,
    provider_tool_call_id: Option<&str>,
    arguments_json: &str,
    mut events: Vec<ToolMediationEvent>,
    error: ModelReadableToolError,
) -> MediatedToolCall {
    push_pipeline_completed(
        &mut events,
        requested_tool_id,
        tool_id,
        "rejected",
        Some(error.error_code.as_str()),
    );
    let arguments = parse_tool_arguments(arguments_json);
    let execution_args = tool_args(&arguments);
    MediatedToolCall {
        provider_tool_call_id: provider_tool_call_id.map(|id| id.to_string()),
        requested_tool_id: requested_tool_id.to_string(),
        tool_id: tool_id.to_string(),
        arguments_json: arguments_json.to_string(),
        arguments,
        execution_args,
        status: ToolMediationStatus::Rejected,
        events,
        repairs: Vec::new(),
        error: Some(error),
    }
}

fn push_pipeline_completed(
    events: &mut Vec<ToolMediationEvent>,
    requested_tool_id: &str,
    resolved_tool_id: &str,
    status: &str,
    reason: Option<&str>,
) {
    let reason_fragment = reason
        .map(|reason| format!(",\"reason\":{}", json_string(reason)))
        .unwrap_or_default();
    events.push(ToolMediationEvent {
        event_type: "tcml.pipeline.completed".to_string(),
        payload_json: format!(
            "{{\"requested_tool\":{},\"resolved_tool\":{},\"status\":{}{},\"stages\":[\"parse\",\"alias\",\"repair\",\"schema_validate\",\"manifest\"]}}",
            json_string(requested_tool_id),
            json_string(resolved_tool_id),
            json_string(status),
            reason_fragment
        ),
    });
}

fn tool_arguments_json_is_object(arguments_json: &str) -> bool {
    let trimmed = arguments_json.trim();
    if trimmed.is_empty() {
        return false;
    }
    if !trimmed.starts_with('{') || !trimmed.ends_with('}') {
        return false;
    }
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for ch in trimmed.chars() {
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0 && !in_string
}

fn normalize_alias_arguments_for_tool(tool_id: &str, arguments: &mut ParsedToolArguments) {
    if tool_id == "file.read" && arguments.path.is_none() {
        arguments.path = arguments
            .root
            .clone()
            .or_else(|| candidate_path_like(arguments.content.as_deref()));
    }
    if matches!(tool_id, "file.list_directory" | "file.list_tree") && arguments.path.is_none() {
        arguments.path = arguments
            .root
            .clone()
            .or_else(|| candidate_path_like(arguments.content.as_deref()));
    }
    if matches!(tool_id, "file.list_directory" | "file.list_tree") && arguments.root.is_none() {
        arguments.root = arguments.path.clone();
    }
    if tool_id == "repo.map" && arguments.root.is_none() {
        arguments.root = arguments
            .path
            .clone()
            .or_else(|| candidate_path_like(arguments.content.as_deref()));
    }
    if tool_id == "search.ripgrep" && arguments.pattern.is_none() {
        arguments.pattern = arguments
            .query
            .clone()
            .or_else(|| candidate_pattern_like(arguments.content.as_deref()));
    }
    if tool_id == "research.csv_profile" && arguments.input_csv.is_none() {
        arguments.input_csv = arguments.path.clone();
    }
    if tool_id == "shell.command" && arguments.command.is_none() {
        arguments.command = arguments.content.clone();
    }
}

fn candidate_path_like(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || value.contains('\n') {
        return None;
    }
    let looks_like_path = value.contains('/')
        || value.starts_with('.')
        || value.ends_with(".md")
        || value.ends_with(".rs")
        || value.ends_with(".toml")
        || value.ends_with(".json")
        || value.ends_with(".csv");
    if looks_like_path {
        Some(value.to_string())
    } else {
        None
    }
}

fn candidate_pattern_like(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() || value.contains('\n') {
        return None;
    }
    Some(value.to_string())
}

fn suggest_tool_replacement(requested: &str) -> Option<String> {
    let key = requested.to_ascii_lowercase();
    if key.contains("list") || key.contains("dir") {
        Some("file.list_directory".to_string())
    } else if key.contains("tree") {
        Some("file.list_tree".to_string())
    } else if key.contains("read") {
        Some("file.read".to_string())
    } else if key.contains("search") || key.contains("grep") {
        Some("search.ripgrep".to_string())
    } else if key.contains("write") {
        Some("file.write".to_string())
    } else if key.contains("shell") || key.contains("command") || key.contains("bash") {
        Some("shell.command".to_string())
    } else {
        None
    }
}

fn arguments_to_json(arguments: &ParsedToolArguments, fallback: &str) -> String {
    let trimmed = fallback.trim();
    let mut parts = Vec::new();
    if let Some(path) = &arguments.path {
        parts.push(format!("\"path\":{}", json_string(path)));
    }
    if let Some(root) = &arguments.root {
        parts.push(format!("\"root\":{}", json_string(root)));
    }
    if let Some(include_hidden) = arguments.include_hidden {
        parts.push(format!("\"include_hidden\":{include_hidden}"));
    }
    if let Some(pattern) = &arguments.pattern {
        parts.push(format!("\"pattern\":{}", json_string(pattern)));
    }
    if let Some(query) = &arguments.query {
        parts.push(format!("\"query\":{}", json_string(query)));
    }
    if let Some(content) = &arguments.content {
        parts.push(format!("\"content\":{}", json_string(content)));
    }
    if let Some(command) = &arguments.command {
        parts.push(format!("\"command\":{}", json_string(command)));
    }
    if let Some(offset) = arguments.offset {
        parts.push(format!("\"offset\":{offset}"));
    }
    if let Some(limit) = arguments.limit {
        parts.push(format!("\"limit\":{limit}"));
    }
    if let Some(max_bytes) = arguments.max_bytes {
        parts.push(format!("\"max_bytes\":{max_bytes}"));
    }
    if let Some(max_results) = arguments.max_results {
        parts.push(format!("\"max_results\":{max_results}"));
    }
    if let Some(max_files) = arguments.max_files {
        parts.push(format!("\"max_files\":{max_files}"));
    }
    if let Some(max_depth) = arguments.max_depth {
        parts.push(format!("\"max_depth\":{max_depth}"));
    }
    if let Some(old_string) = &arguments.old_string {
        parts.push(format!("\"old_string\":{}", json_string(old_string)));
    }
    if let Some(new_string) = &arguments.new_string {
        parts.push(format!("\"new_string\":{}", json_string(new_string)));
    }
    if let Some(base_hash) = &arguments.base_hash {
        parts.push(format!("\"base_hash\":{}", json_string(base_hash)));
    }
    if let Some(replace_all) = arguments.replace_all {
        parts.push(format!("\"replace_all\":{replace_all}"));
    }
    if let Some(edits_json) = &arguments.edits_json {
        parts.push(format!(
            "\"edits\":{}",
            if is_complete_json_value(edits_json) {
                edits_json.clone()
            } else {
                json_string(edits_json)
            }
        ));
    }
    if let Some(input_csv) = &arguments.input_csv {
        parts.push(format!("\"input_csv\":{}", json_string(input_csv)));
    }
    if let Some(job_id) = &arguments.job_id {
        parts.push(format!("\"job_id\":{}", json_string(job_id)));
    }
    if let Some(answer) = &arguments.answer {
        parts.push(format!("\"answer\":{}", json_string(answer)));
    }
    if let Some(model_role) = &arguments.model_role {
        parts.push(format!("\"model_role\":{}", json_string(model_role)));
    }
    if let Some(write_scope_json) = &arguments.write_scope_json {
        parts.push(format!(
            "\"write_scope\":{}",
            if is_complete_json_value(write_scope_json) {
                write_scope_json.clone()
            } else {
                json_string(write_scope_json)
            }
        ));
    }
    if parts.is_empty() && trimmed.starts_with('{') && trimmed.ends_with('}') {
        return trimmed.to_string();
    }
    format!("{{{}}}", parts.join(","))
}

fn is_complete_json_value(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'))
}

pub fn tool_args(arguments: &ParsedToolArguments) -> ToolExecutionArgs {
    ToolExecutionArgs {
        path: arguments.path.clone(),
        root: arguments.root.clone().or_else(|| Some(".".to_string())),
        include_hidden: arguments.include_hidden,
        command: arguments.command.clone(),
        content: arguments.content.clone(),
        pattern: arguments
            .pattern
            .clone()
            .or_else(|| arguments.query.clone()),
        query: arguments.query.clone(),
        old_string: arguments.old_string.clone(),
        new_string: arguments.new_string.clone(),
        base_hash: arguments.base_hash.clone(),
        replace_all: arguments.replace_all,
        offset: arguments.offset,
        limit: arguments.limit,
        max_bytes: arguments.max_bytes,
        max_results: arguments.max_results,
        max_files: arguments.max_files,
        max_depth: arguments.max_depth,
        edits_json: arguments.edits_json.clone(),
        input_csv: arguments.input_csv.clone(),
        job_id: arguments.job_id.clone(),
        answer: arguments.answer.clone(),
        model_role: arguments.model_role.clone(),
        write_scope_json: arguments.write_scope_json.clone(),
        ..ToolExecutionArgs::default()
    }
}

fn json_optional_string(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_string())
}

fn json_array_strings(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| json_string(value))
            .collect::<Vec<_>>()
            .join(",")
    )
}

fn json_string(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other if other.is_control() => {
                escaped.push_str(&format!("\\u{:04x}", other as u32));
            }
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_alias_to_canonical_tool() {
        let call = mediate_tool_call("read_source_code", r#"{"path":"README.md"}"#);
        assert_eq!(call.tool_id, "file.read");
        assert!(call.error.is_none());
        assert!(call
            .events
            .iter()
            .any(|event| event.event_type == "tool.name.resolution_started"));
        assert!(call
            .events
            .iter()
            .any(|event| event.event_type == "tool.name.alias_resolved"));
        assert!(call.events.iter().any(|event| {
            event.event_type == "tcml.pipeline.completed"
                && event.payload_json.contains("\"status\":\"ready\"")
        }));
    }

    #[test]
    fn mediation_preserves_provider_tool_call_id() {
        let call = mediate_tool_call_with_provider_id(
            "file_read",
            Some("call_provider_readme"),
            r#"{"path":"README.md"}"#,
        );
        assert_eq!(
            call.provider_tool_call_id.as_deref(),
            Some("call_provider_readme")
        );
        assert_eq!(call.tool_id, "file.read");
        assert!(call.events.iter().any(|event| {
            event.event_type == "tool.name.resolution_started"
                && event
                    .payload_json
                    .contains("\"provider_tool_call_id\":\"call_provider_readme\"")
        }));
    }

    #[test]
    fn unknown_tool_is_model_readable_error() {
        let call = mediate_tool_call("made_up_reader", r#"{"path":"README.md"}"#);
        assert_eq!(call.status, ToolMediationStatus::Rejected);
        assert_eq!(call.error.unwrap().error_code, "UNKNOWN_TOOL");
        assert!(call
            .events
            .iter()
            .any(|event| event.event_type == "tool.name.unknown"));
        assert!(call
            .events
            .iter()
            .any(|event| event.event_type == "tool.doctor.snapshot"));
        assert!(call
            .events
            .iter()
            .any(|event| event.event_type == "tool.call.rejected"));
        assert!(call.events.iter().any(|event| {
            event.event_type == "tool.mediation.completed"
                && event.payload_json.contains("\"reason\":\"unknown_tool\"")
        }));
        assert!(call.events.iter().any(|event| {
            event.event_type == "tcml.pipeline.completed"
                && event.payload_json.contains("\"status\":\"rejected\"")
        }));
    }

    #[test]
    fn malformed_tool_json_is_model_readable_error_even_without_required_fields() {
        let call = mediate_tool_call("plan_exit", r#"{"unexpected": true"#);
        assert_eq!(call.status, ToolMediationStatus::Rejected);
        assert_eq!(
            call.error.as_ref().unwrap().error_code,
            "MALFORMED_TOOL_JSON"
        );
        assert!(call.events.iter().any(|event| {
            event.event_type == "tool.mediation.completed"
                && event
                    .payload_json
                    .contains("\"reason\":\"malformed_tool_json\"")
        }));
    }

    #[test]
    fn file_read_limit_defaults_offset() {
        let call = mediate_tool_call("file_read", r#"{"path":"README.md","limit":2000}"#);
        assert_eq!(call.arguments.offset, Some(0));
        assert!(call
            .events
            .iter()
            .any(|event| event.event_type == "tool.relational_default_applied"));
        assert!(call.events.iter().any(|event| {
            event.event_type == "tool.mediation.completed"
                && event.payload_json.contains("\"status\":\"repaired\"")
        }));
        assert!(call.events.iter().any(|event| {
            event.event_type == "tcml.pipeline.completed"
                && event.payload_json.contains("\"status\":\"repaired\"")
        }));
    }

    #[test]
    fn file_read_string_limit_uses_tcml_relational_default() {
        let call = mediate_tool_call("file_read", r#"{"path":"README.md","limit":"2000"}"#);
        assert_eq!(call.status, ToolMediationStatus::Repaired);
        assert_eq!(call.arguments.limit, Some(2000));
        assert_eq!(call.arguments.offset, Some(0));
        assert!(call.arguments_json.contains("\"limit\":2000"));
        assert!(call.arguments_json.contains("\"offset\":0"));
        assert!(!call.arguments_json.contains("\"limit\":\"2000\""));
        assert!(call.repairs.iter().any(|repair| {
            repair.issue_path == "offset" && repair.repair_rule == "default_offset_for_limited_read"
        }));
    }

    #[test]
    fn file_read_accepts_filepath_alias_argument() {
        let call = mediate_tool_call("read", r#"{"filePath":"README.md","maxBytes":"4096"}"#);
        assert!(call.error.is_none(), "{call:?}");
        assert_eq!(call.tool_id, "file.read");
        assert_eq!(call.arguments.path.as_deref(), Some("README.md"));
        assert_eq!(call.arguments.max_bytes, Some(4096));
    }

    #[test]
    fn todo_write_accepts_items_array() {
        let call = mediate_tool_call(
            "todo_write",
            r#"{"items":[{"content":"inspect runtime","status":"in_progress"}]}"#,
        );
        assert!(call.error.is_none(), "{call:?}");
        assert_eq!(call.tool_id, "todo.write");
        assert!(call
            .arguments
            .content
            .as_deref()
            .unwrap()
            .contains("inspect runtime"));
    }

    #[test]
    fn file_write_content_is_not_repaired() {
        let call = mediate_tool_call("file_write", r#"{"path":"x.html","content":null}"#);
        assert_eq!(call.status, ToolMediationStatus::Rejected);
        assert!(!call
            .repairs
            .iter()
            .any(|repair| repair.issue_path == "content"));
    }

    #[test]
    fn file_write_path_is_not_silently_markdown_repaired() {
        let call = mediate_tool_call("file_write", r#"{"path":"[x](x.html)","content":"ok"}"#);
        assert_eq!(call.status, ToolMediationStatus::Ready);
        assert_eq!(call.arguments.path.as_deref(), Some("[x](x.html)"));
        assert!(!call
            .repairs
            .iter()
            .any(|repair| repair.issue_path == "path"));
    }

    #[test]
    fn file_write_create_does_not_require_base_hash() {
        let call = mediate_tool_call("file_write", r#"{"path":"x.html","content":"ok"}"#);
        assert_eq!(call.status, ToolMediationStatus::Ready);
        assert_eq!(call.tool_id, "file.write");
    }

    #[test]
    fn patch_apply_does_not_require_model_supplied_base_hash() {
        let call = mediate_tool_call(
            "patch.propose",
            r#"{"path":"src/lib.rs","old_string":"a","new_string":"b"}"#,
        );
        assert_eq!(call.status, ToolMediationStatus::Ready, "{:?}", call.error);
        assert_eq!(call.tool_id, "patch.apply");
        assert_eq!(call.arguments.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(call.arguments.old_string.as_deref(), Some("a"));
        assert_eq!(call.arguments.new_string.as_deref(), Some("b"));
        assert_eq!(call.arguments.base_hash, None);
    }

    #[test]
    fn file_edit_does_not_require_model_supplied_base_hash() {
        let call = mediate_tool_call(
            "file_edit",
            r#"{"path":"src/lib.rs","old_string":"a","new_string":"b"}"#,
        );
        assert_eq!(call.status, ToolMediationStatus::Ready, "{:?}", call.error);
        assert_eq!(call.tool_id, "file.edit");
        assert_eq!(call.arguments.path.as_deref(), Some("src/lib.rs"));
        assert_eq!(call.arguments.old_string.as_deref(), Some("a"));
        assert_eq!(call.arguments.new_string.as_deref(), Some("b"));
        assert_eq!(call.arguments.base_hash, None);
    }

    #[test]
    fn file_multi_edit_does_not_require_model_supplied_base_hash() {
        let call = mediate_tool_call(
            "file_multi_edit",
            r#"{"path":"src/lib.rs","edits":[{"old_string":"a","new_string":"b"}]}"#,
        );
        assert_eq!(call.status, ToolMediationStatus::Ready, "{:?}", call.error);
        assert_eq!(call.tool_id, "file.multi_edit");
        assert_eq!(call.arguments.path.as_deref(), Some("src/lib.rs"));
        assert!(call.arguments.edits_json.is_some());
        assert_eq!(call.arguments.base_hash, None);
    }

    #[test]
    fn shell_command_is_not_repaired() {
        let call = mediate_tool_call("shell_command", r#"{"command":null}"#);
        assert_eq!(call.status, ToolMediationStatus::Rejected);
        assert!(!call
            .repairs
            .iter()
            .any(|repair| repair.issue_path == "command"));
    }

    #[test]
    fn ledger_requires_exactly_once_results() {
        let mut ledger = ToolCallLedger::default();
        ledger.propose("a");
        ledger.propose("b");
        assert!(ledger.record_result("a"));
        assert!(!ledger.record_result("a"));
        assert!(!ledger.exactly_once_ok());
        assert_eq!(ledger.missing_results(), vec!["b".to_string()]);
        assert_eq!(ledger.duplicate_results(), vec!["a".to_string()]);
    }

    #[test]
    fn content_tool_candidate_uses_existing_parser() {
        let raw = r#"<｜｜DSML｜｜tool_calls><｜｜DSML｜｜invoke name="file_read"><｜｜DSML｜｜parameter name="path" string="true">README.md</｜｜DSML｜｜parameter></｜｜DSML｜｜invoke></｜｜DSML｜｜tool_calls>"#;
        let candidates = extract_content_tool_call_candidates(raw);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].tool_id, "file_read");
    }
}
