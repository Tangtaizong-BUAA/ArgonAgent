//! TCML schema validation facade.
//!
//! The required-argument validation implementation runs through the TCML
//! contract mediator. This facade gives callers a named schema-validation
//! concept while preserving the existing mediation behavior.

use crate::tcml::{mediate_tool_call, MediatedToolCall, ParsedToolArguments, ToolMediationStatus};
use researchcode_kernel::tool::ToolSpec;

#[derive(Debug, Clone, Default)]
pub struct SchemaValidator;

impl SchemaValidator {
    pub fn validate(requested_tool_id: &str, arguments_json: &str) -> MediatedToolCall {
        mediate_tool_call(requested_tool_id, arguments_json)
    }

    pub fn is_schema_rejected(call: &MediatedToolCall) -> bool {
        call.status == ToolMediationStatus::Rejected
            && call
                .error
                .as_ref()
                .is_some_and(|error| error.error_code == "SCHEMA_VALIDATION_FAILED")
    }
}

pub fn validate_required_arguments(
    spec: &ToolSpec,
    arguments: &ParsedToolArguments,
) -> Vec<String> {
    required_keys_for_tool(&spec.tool_id)
        .into_iter()
        .filter(|key| !has_argument(arguments, key))
        .map(|key| format!("missing required field {key}"))
        .collect()
}

pub fn required_keys_for_tool(tool_id: &str) -> Vec<&'static str> {
    match tool_id {
        "file.read" => vec!["path"],
        "file.list_directory" | "file.list_tree" => vec!["path"],
        "file.edit" => vec!["path", "old_string", "new_string"],
        "file.write" => vec!["path", "content"],
        "file.multi_edit" => vec!["path", "edits"],
        "search.ripgrep" => vec!["pattern"],
        "shell.command" => vec!["command"],
        "patch.apply" => vec!["path", "old_string", "new_string"],
        "todo.write" => vec!["items"],
        "plan.enter" => vec!["plan"],
        "plan.write" => vec!["content"],
        "ask_user" => vec!["question"],
        "task.dispatch" => vec!["prompt"],
        "research.csv_profile" => vec!["input_csv"],
        _ => Vec::new(),
    }
}

pub fn is_required_key(spec: &ToolSpec, key: &str) -> bool {
    required_keys_for_tool(&spec.tool_id).contains(&key)
}

fn has_argument(arguments: &ParsedToolArguments, key: &str) -> bool {
    match key {
        "path" => present(arguments.path.as_deref()),
        "root" => present(arguments.root.as_deref()),
        "command" => present(arguments.command.as_deref()),
        "pattern" => present(arguments.pattern.as_deref()),
        "query" => present(arguments.query.as_deref()),
        "content" | "question" | "plan" | "items" | "prompt" => {
            present(arguments.content.as_deref())
        }
        "include_hidden" => arguments.include_hidden.is_some(),
        "old_string" => arguments.old_string.is_some(),
        "new_string" => arguments.new_string.is_some(),
        "base_hash" => present(arguments.base_hash.as_deref()),
        "edits" => present(arguments.edits_json.as_deref()),
        "input_csv" => present(arguments.input_csv.as_deref()),
        _ => true,
    }
}

fn present(value: Option<&str>) -> bool {
    value.map(|value| !value.is_empty()).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_validator_reports_missing_required_fields() {
        let call = SchemaValidator::validate("file_edit", r#"{"path":"src/lib.rs"}"#);
        assert!(SchemaValidator::is_schema_rejected(&call));
        assert!(call
            .error
            .as_ref()
            .unwrap()
            .short_message
            .contains("missing required field old_string"));
    }

    #[test]
    fn required_argument_validator_reports_all_missing_fields() {
        let spec = researchcode_kernel::tool::find_tool_spec("file.edit").unwrap();
        let arguments = ParsedToolArguments {
            path: Some("src/lib.rs".to_string()),
            ..ParsedToolArguments::default()
        };
        assert_eq!(
            validate_required_arguments(&spec, &arguments),
            vec![
                "missing required field old_string".to_string(),
                "missing required field new_string".to_string()
            ]
        );
    }
}
