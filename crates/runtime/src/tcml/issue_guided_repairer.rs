//! TCML issue-guided repair facade.
//!
//! Repairs execute inside the TCML contract mediator, and callers can observe
//! the repaired outcome through this TCML-owned API.

use crate::tcml::{
    can_repair_field, file_read_relational_default, markdown_link_path_target, mediate_tool_call,
    optional_null_present, quoted_usize_argument, FileReadRelationalDefault, MediatedToolCall,
    ParsedToolArguments, ToolInputRepair, ToolMediationEvent, OPTIONAL_NULL_REPAIR_KEYS,
};
use researchcode_kernel::tool::ToolSpec;
use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct IssueGuidedRepairer;

impl IssueGuidedRepairer {
    pub fn repair(requested_tool_id: &str, arguments_json: &str) -> MediatedToolCall {
        mediate_tool_call(requested_tool_id, arguments_json)
    }

    pub fn repairs(call: &MediatedToolCall) -> &[ToolInputRepair] {
        &call.repairs
    }
}

pub fn apply_low_risk_repairs(
    spec: &ToolSpec,
    original_json: &str,
    arguments: &mut ParsedToolArguments,
    events: &mut Vec<ToolMediationEvent>,
    repairs: &mut Vec<ToolInputRepair>,
) {
    for key in ["path", "root", "input_csv"] {
        if !can_repair_field(&spec.tool_id, key) {
            continue;
        }
        if let Some(repaired) = markdown_link_path_target(match key {
            "path" => arguments.path.as_deref(),
            "root" => arguments.root.as_deref(),
            "input_csv" => arguments.input_csv.as_deref(),
            _ => None,
        }) {
            let before = match key {
                "path" => arguments.path.replace(repaired.clone()).unwrap_or_default(),
                "root" => arguments.root.replace(repaired.clone()).unwrap_or_default(),
                "input_csv" => arguments
                    .input_csv
                    .replace(repaired.clone())
                    .unwrap_or_default(),
                _ => String::new(),
            };
            push_repair(
                spec,
                key,
                "markdown_path_unwrap",
                &before,
                &repaired,
                events,
                repairs,
            );
        }
    }
    if spec.tool_id == "file.read" && arguments.limit.is_none() {
        if let Some(limit) = quoted_usize_argument(original_json, "limit") {
            arguments.limit = Some(limit);
            push_repair(
                spec,
                "limit",
                "quoted_integer_to_integer",
                "string",
                &limit.to_string(),
                events,
                repairs,
            );
        }
    }
    if spec.tool_id == "file.read" && arguments.offset.is_none() {
        if let Some(offset) = quoted_usize_argument(original_json, "offset") {
            arguments.offset = Some(offset);
            push_repair(
                spec,
                "offset",
                "quoted_integer_to_integer",
                "string",
                &offset.to_string(),
                events,
                repairs,
            );
        }
    }
    if spec.tool_id == "file.read" && arguments.max_bytes.is_none() {
        if let Some(max_bytes) = quoted_usize_argument(original_json, "max_bytes") {
            arguments.max_bytes = Some(max_bytes);
            push_repair(
                spec,
                "max_bytes",
                "quoted_integer_to_integer",
                "string",
                &max_bytes.to_string(),
                events,
                repairs,
            );
        }
    }
    if spec.tool_id == "file.read" {
        apply_file_read_relational_default(spec, arguments, events, repairs);
    }
    apply_array_shape_repairs(spec, arguments, events, repairs);
    for optional_key in OPTIONAL_NULL_REPAIR_KEYS {
        if optional_null_present(original_json, optional_key)
            && !crate::tcml::is_required_key(spec, optional_key)
        {
            push_repair(
                spec,
                optional_key,
                "strip_optional_null",
                "null",
                "missing",
                events,
                repairs,
            );
        }
    }
}

fn apply_array_shape_repairs(
    spec: &ToolSpec,
    arguments: &mut ParsedToolArguments,
    events: &mut Vec<ToolMediationEvent>,
    repairs: &mut Vec<ToolInputRepair>,
) {
    if spec.tool_id == "todo.write" {
        let Some(items) = arguments.content.clone() else {
            return;
        };
        if let Some((repaired, rule)) = repair_array_value(&items, true) {
            arguments.content = Some(repaired.clone());
            push_repair(
                spec,
                "items",
                rule,
                summarize_json(&items).as_str(),
                &repaired,
                events,
                repairs,
            );
        }
        return;
    }

    if spec.tool_id == "task.dispatch" {
        let Some(scope_json) = arguments.write_scope_json.clone() else {
            return;
        };
        if let Some((repaired, rule)) = repair_write_scope_paths(&scope_json) {
            arguments.write_scope_json = Some(repaired.clone());
            push_repair(
                spec,
                "write_scope.paths",
                rule,
                summarize_json(&scope_json).as_str(),
                &repaired,
                events,
                repairs,
            );
        }
    }
}

fn repair_array_value(value: &str, allow_bare_string: bool) -> Option<(String, &'static str)> {
    let parsed: Value = match serde_json::from_str(value) {
        Ok(parsed) => parsed,
        Err(_) if allow_bare_string && !value.trim().is_empty() => {
            return Some((
                Value::Array(vec![Value::String(value.to_string())]).to_string(),
                "wrap_bare_string_to_array",
            ));
        }
        Err(_) => return None,
    };
    match parsed {
        Value::Array(_) => None,
        Value::String(inner) => {
            if let Ok(Value::Array(array)) = serde_json::from_str::<Value>(&inner) {
                return Some((Value::Array(array).to_string(), "parse_stringified_array"));
            }
            if allow_bare_string && !inner.trim().is_empty() {
                return Some((
                    Value::Array(vec![Value::String(inner)]).to_string(),
                    "wrap_bare_string_to_array",
                ));
            }
            None
        }
        Value::Object(map) if map.is_empty() => Some(("[]".to_string(), "empty_object_to_array")),
        _ => None,
    }
}

fn repair_write_scope_paths(scope_json: &str) -> Option<(String, &'static str)> {
    let mut scope: Value = serde_json::from_str(scope_json).ok()?;
    let Value::Object(map) = &mut scope else {
        return None;
    };
    let paths = map.get("paths")?.clone();
    let (repaired_paths, rule) = match paths {
        Value::Array(_) => return None,
        Value::String(inner) => {
            if let Ok(Value::Array(array)) = serde_json::from_str::<Value>(&inner) {
                (Value::Array(array), "parse_stringified_array")
            } else if inner.trim().is_empty() {
                return None;
            } else {
                (
                    Value::Array(vec![Value::String(inner)]),
                    "wrap_bare_string_to_array",
                )
            }
        }
        Value::Object(map) if map.is_empty() => (Value::Array(Vec::new()), "empty_object_to_array"),
        _ => return None,
    };
    map.insert("paths".to_string(), repaired_paths);
    Some((scope.to_string(), rule))
}

fn summarize_json(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.len() > 80 {
        format!("{}...", &trimmed[..80])
    } else {
        trimmed.to_string()
    }
}

fn apply_file_read_relational_default(
    spec: &ToolSpec,
    arguments: &mut ParsedToolArguments,
    events: &mut Vec<ToolMediationEvent>,
    repairs: &mut Vec<ToolInputRepair>,
) {
    let Some(default) = file_read_relational_default(arguments.limit, arguments.offset) else {
        return;
    };
    match default {
        FileReadRelationalDefault::OffsetForLimitedRead { offset } => {
            arguments.offset = Some(offset);
        }
        FileReadRelationalDefault::LimitForOffsetRead { limit } => {
            arguments.limit = Some(limit);
        }
    }
    let value = default.value();
    events.push(ToolMediationEvent {
        event_type: "tool.relational_default_applied".to_string(),
        payload_json: format!(
            "{{\"tool_id\":{},\"issue_path\":{},\"default\":{},\"reason\":{}}}",
            json_string(&spec.tool_id),
            json_string(default.issue_path()),
            value,
            json_string(default.reason())
        ),
    });
    push_repair(
        spec,
        default.issue_path(),
        default.repair_rule(),
        "missing",
        &value.to_string(),
        events,
        repairs,
    );
}

fn push_repair(
    spec: &ToolSpec,
    issue_path: &str,
    repair_rule: &str,
    before: &str,
    after: &str,
    events: &mut Vec<ToolMediationEvent>,
    repairs: &mut Vec<ToolInputRepair>,
) {
    if !can_repair_field(&spec.tool_id, issue_path) {
        return;
    }
    let repair = ToolInputRepair {
        tool_name: spec.tool_id.clone(),
        issue_path: issue_path.to_string(),
        repair_rule: repair_rule.to_string(),
        before_summary: before.to_string(),
        after_summary: after.to_string(),
        confidence: "high".to_string(),
    };
    events.push(ToolMediationEvent {
        event_type: "tool.input_repaired".to_string(),
        payload_json: format!(
            "{{\"tool_id\":{},\"issue_path\":{},\"repair_rule\":{},\"confidence\":\"high\"}}",
            json_string(&repair.tool_name),
            json_string(&repair.issue_path),
            json_string(&repair.repair_rule)
        ),
    });
    repairs.push(repair);
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
            other => escaped.push(other),
        }
    }
    format!("\"{escaped}\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_guided_repairer_exposes_relational_repairs() {
        let call = IssueGuidedRepairer::repair("file_read", r#"{"path":"README.md","limit":"20"}"#);
        assert!(IssueGuidedRepairer::repairs(&call)
            .iter()
            .any(|repair| repair.issue_path == "offset"
                && repair.repair_rule == "default_offset_for_limited_read"));
    }

    #[test]
    fn low_risk_repairer_mutates_arguments_and_records_events() {
        let spec = researchcode_kernel::tool::find_tool_spec("file.read").unwrap();
        let mut arguments = ParsedToolArguments {
            path: Some("[README](README.md)".to_string()),
            limit: Some(20),
            ..ParsedToolArguments::default()
        };
        let mut events = Vec::new();
        let mut repairs = Vec::new();
        apply_low_risk_repairs(
            &spec,
            r#"{"path":"[README](README.md)","limit":20}"#,
            &mut arguments,
            &mut events,
            &mut repairs,
        );
        assert_eq!(arguments.path.as_deref(), Some("README.md"));
        assert_eq!(arguments.offset, Some(0));
        assert!(events
            .iter()
            .any(|event| event.event_type == "tool.input_repaired"));
        assert!(repairs
            .iter()
            .any(|repair| repair.repair_rule == "markdown_path_unwrap"));
    }

    #[test]
    fn repairs_bare_todo_item_to_array() {
        let call = IssueGuidedRepairer::repair("todo_write", r#"{"items":"inspect runtime"}"#);

        assert_eq!(call.tool_id, "todo.write");
        assert_eq!(
            call.execution_args.content.as_deref(),
            Some(r#"["inspect runtime"]"#)
        );
        assert!(call.repairs.iter().any(|repair| {
            repair.issue_path == "items" && repair.repair_rule == "wrap_bare_string_to_array"
        }));
    }

    #[test]
    fn repairs_empty_object_items_to_array() {
        let call = IssueGuidedRepairer::repair("todo_write", r#"{"items":{}}"#);

        assert_eq!(call.execution_args.content.as_deref(), Some("[]"));
        assert!(call.repairs.iter().any(|repair| {
            repair.issue_path == "items" && repair.repair_rule == "empty_object_to_array"
        }));
    }

    #[test]
    fn repairs_task_dispatch_write_scope_paths_array_shapes() {
        let stringified = IssueGuidedRepairer::repair(
            "task_dispatch",
            r#"{"prompt":"edit scoped file","model_role":"executor","write_scope":{"paths":"[\"src\",\"tests\"]"}}"#,
        );
        assert_eq!(
            stringified.arguments.write_scope_json.as_deref(),
            Some(r#"{"paths":["src","tests"]}"#)
        );
        assert!(stringified.repairs.iter().any(|repair| {
            repair.issue_path == "write_scope.paths"
                && repair.repair_rule == "parse_stringified_array"
        }));

        let bare = IssueGuidedRepairer::repair(
            "task_dispatch",
            r#"{"prompt":"edit scoped file","model_role":"executor","write_scope":{"paths":"src"}}"#,
        );
        assert_eq!(
            bare.arguments.write_scope_json.as_deref(),
            Some(r#"{"paths":["src"]}"#)
        );
        assert!(bare.repairs.iter().any(|repair| {
            repair.issue_path == "write_scope.paths"
                && repair.repair_rule == "wrap_bare_string_to_array"
        }));
    }
}
