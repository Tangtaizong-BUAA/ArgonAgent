use crate::tcml::ModelReadableToolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolErrorCode {
    UnknownTool,
    MalformedToolJson,
    SchemaValidationFailed,
    ToolNotInManifest,
    WrongToolIntent,
    PreToolUseDenied,
    PermissionDenied,
    SensitivePath,
    PathEscapesWorkspace,
    CommandClassifierBlocked,
    ToolFailed,
    ToolTimeout,
}

impl ToolErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            ToolErrorCode::UnknownTool => "UNKNOWN_TOOL",
            ToolErrorCode::MalformedToolJson => "MALFORMED_TOOL_JSON",
            ToolErrorCode::SchemaValidationFailed => "SCHEMA_VALIDATION_FAILED",
            ToolErrorCode::ToolNotInManifest => "TOOL_NOT_IN_MANIFEST",
            ToolErrorCode::WrongToolIntent => "WRONG_TOOL_INTENT",
            ToolErrorCode::PreToolUseDenied => "PRE_TOOL_USE_DENIED",
            ToolErrorCode::PermissionDenied => "PERMISSION_DENIED",
            ToolErrorCode::SensitivePath => "SENSITIVE_PATH",
            ToolErrorCode::PathEscapesWorkspace => "PATH_ESCAPES_WORKSPACE",
            ToolErrorCode::CommandClassifierBlocked => "COMMAND_CLASSIFIER_BLOCKED",
            ToolErrorCode::ToolFailed => "TOOL_FAILED",
            ToolErrorCode::ToolTimeout => "TOOL_TIMEOUT",
        }
    }
}

pub fn model_readable_tool_error(
    code: ToolErrorCode,
    tool_name: impl Into<String>,
    short_message: impl Into<String>,
    retryable: bool,
    suggested_replacement: Option<String>,
) -> ModelReadableToolError {
    ModelReadableToolError {
        error_code: code.as_str().to_string(),
        tool_name: tool_name.into(),
        short_message: short_message.into(),
        field_errors: Vec::new(),
        retryable,
        retry_hint: None,
        retry_example: None,
        counts_against_budget: true,
        suggested_replacement,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_doc39_error_codes() {
        assert_eq!(ToolErrorCode::UnknownTool.as_str(), "UNKNOWN_TOOL");
        assert_eq!(
            ToolErrorCode::PermissionDenied.as_str(),
            "PERMISSION_DENIED"
        );
        assert_eq!(
            ToolErrorCode::ToolNotInManifest.as_str(),
            "TOOL_NOT_IN_MANIFEST"
        );
        assert_eq!(ToolErrorCode::WrongToolIntent.as_str(), "WRONG_TOOL_INTENT");
        assert_eq!(
            ToolErrorCode::PreToolUseDenied.as_str(),
            "PRE_TOOL_USE_DENIED"
        );
        assert_eq!(
            ToolErrorCode::CommandClassifierBlocked.as_str(),
            "COMMAND_CLASSIFIER_BLOCKED"
        );
        assert_eq!(ToolErrorCode::ToolTimeout.as_str(), "TOOL_TIMEOUT");
    }
}
