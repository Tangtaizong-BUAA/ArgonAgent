use researchcode_kernel::PermissionRequestType;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionMode {
    Default,
    Plan,
    AcceptEdits,
    DontAsk,
    BypassPermissions,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionDecision {
    Allow,
    Ask { reason: String },
    Deny { reason: String },
}

#[derive(Debug, Default, Clone)]
pub struct PermissionPolicy;

/// Mode fallback used by the unified permission resolver after project rules,
/// tool-specific checks, and safety sentinels have had first refusal. Production
/// callers should enter through `PermissionGate::evaluate` so path, args,
/// request type, and persisted policy rules are all considered.
#[derive(Debug, Clone)]
pub struct PermissionEvaluationRequest<'a> {
    pub mode: PermissionMode,
    pub tool_id: &'a str,
    pub args: &'a serde_json::Value,
    pub request_type: PermissionRequestType,
    pub session_id: &'a str,
    pub command_summary: Option<&'a str>,
}

impl PermissionPolicy {
    pub fn evaluate(request: &PermissionEvaluationRequest<'_>) -> PermissionDecision {
        let tool_id = request.tool_id;
        match request.mode {
            PermissionMode::BypassPermissions => PermissionDecision::Allow,
            PermissionMode::Plan if is_state_changing(tool_id) => PermissionDecision::Deny {
                reason: "state-changing tool requires plan approval".to_string(),
            },
            PermissionMode::AcceptEdits if is_file_edit(tool_id) => PermissionDecision::Allow,
            PermissionMode::AcceptEdits if is_state_changing(tool_id) => PermissionDecision::Ask {
                reason: "non-edit state-changing tool requires permission".to_string(),
            },
            PermissionMode::DontAsk if is_read_only(tool_id) => PermissionDecision::Allow,
            PermissionMode::DontAsk => PermissionDecision::Deny {
                reason: "tool is not in the dont-ask allowlist".to_string(),
            },
            PermissionMode::Default if is_state_changing(tool_id) => PermissionDecision::Ask {
                reason: "state-changing tool requires permission".to_string(),
            },
            _ => PermissionDecision::Allow,
        }
    }
}

fn is_read_only(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "file.read"
            | "file.list_directory"
            | "file.list_tree"
            | "repo.map"
            | "search.ripgrep"
            | "git.status"
    )
}

fn is_file_edit(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "file.write" | "file.edit" | "file.multi_edit" | "patch.apply"
    )
}

fn is_state_changing(tool_id: &str) -> bool {
    is_file_edit(tool_id) || matches!(tool_id, "shell.command" | "powershell.command")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request<'a>(
        mode: PermissionMode,
        tool_id: &'a str,
        args: &'a serde_json::Value,
    ) -> PermissionEvaluationRequest<'a> {
        PermissionEvaluationRequest {
            mode,
            tool_id,
            args,
            request_type: PermissionRequestType::FileWrite,
            session_id: "sess",
            command_summary: None,
        }
    }

    #[test]
    fn default_mode_asks_for_shell() {
        let args = serde_json::Value::Null;
        assert!(matches!(
            PermissionPolicy::evaluate(&request(PermissionMode::Default, "shell.command", &args)),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn plan_mode_denies_write() {
        let args = serde_json::Value::Null;
        assert!(matches!(
            PermissionPolicy::evaluate(&request(PermissionMode::Plan, "file.write", &args)),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn accept_edits_does_not_auto_allow_shell() {
        let args = serde_json::Value::Null;
        assert!(matches!(
            PermissionPolicy::evaluate(&request(PermissionMode::AcceptEdits, "file.edit", &args)),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            PermissionPolicy::evaluate(&request(
                PermissionMode::AcceptEdits,
                "shell.command",
                &args
            )),
            PermissionDecision::Ask { .. }
        ));
    }

    #[test]
    fn dont_ask_only_allows_read_only_tools() {
        let args = serde_json::Value::Null;
        assert!(matches!(
            PermissionPolicy::evaluate(&request(PermissionMode::DontAsk, "file.read", &args)),
            PermissionDecision::Allow
        ));
        assert!(matches!(
            PermissionPolicy::evaluate(&request(PermissionMode::DontAsk, "patch.apply", &args)),
            PermissionDecision::Deny { .. }
        ));
    }

    #[test]
    fn evaluation_request_carries_call_context() {
        let args = json!({"path":"src/lib.rs"});
        let request = PermissionEvaluationRequest {
            mode: PermissionMode::AcceptEdits,
            tool_id: "file.edit",
            args: &args,
            request_type: PermissionRequestType::FileWrite,
            session_id: "sess_context",
            command_summary: Some("edit src/lib.rs"),
        };
        assert_eq!(request.args["path"], "src/lib.rs");
        assert_eq!(request.session_id, "sess_context");
        assert!(matches!(
            PermissionPolicy::evaluate(&request),
            PermissionDecision::Allow
        ));
    }
}
