use crate::agent_kernel::permission_gate::{
    DefaultTool, FileEditTool, FileWriteTool, PatchApplyTool, ShellCommandTool,
};
use crate::agent_kernel::{PermissionGate, PermissionMode};
use crate::permission_policy::{
    permission_rule_from_decision, PermissionCheck, PermissionRequest, PermissionResolution,
    PermissionRuleScope, PermissionRuleSet, PermissionRuleStore,
};
use crate::runtime::session_store::RuntimeSessionRecord;
use crate::tool_execution::ToolExecutionArgs;
use researchcode_kernel::{Actor, PermissionDecisionKind, PermissionRequestType};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct PermissionService {
    rule_store: Arc<PermissionRuleStore>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FacadeToolMode {
    Preview,
    FastAutoApply,
    RequirePermission(PermissionRequestType),
    Blocked(String),
}

impl PermissionService {
    pub fn new(policy_path: impl Into<PathBuf>) -> Self {
        Self {
            rule_store: Arc::new(PermissionRuleStore::new(policy_path)),
        }
    }

    pub fn rule_store(&self) -> Arc<PermissionRuleStore> {
        self.rule_store.clone()
    }

    pub fn new_gate(
        &self,
        session_id: &str,
        mode: PermissionMode,
        inline_policy: PermissionRuleSet,
        workspace_root: impl Into<String>,
    ) -> PermissionGate {
        PermissionGate::new(
            self.rule_store.clone(),
            inline_policy,
            mode,
            workspace_root,
            session_id.to_string(),
        )
    }

    pub(crate) fn apply_permission_policy(
        &self,
        record: &mut RuntimeSessionRecord,
        mode: FacadeToolMode,
        tool_id: &str,
        args: &ToolExecutionArgs,
    ) -> Result<FacadeToolMode, String> {
        if matches!(mode, FacadeToolMode::Preview | FacadeToolMode::Blocked(_)) {
            return Ok(mode);
        }
        let Some(request_type) = permission_request_type_for_tool(tool_id) else {
            return Ok(mode);
        };
        let resolver_mode = permission_mode_for_facade_tool_mode(&mode);
        let normalized_summary = normalized_permission_summary(tool_id, args);
        let tool = facade_permission_tool_for_id(tool_id);
        let args_json = facade_permission_args_json(args);
        let request = PermissionRequest {
            mode: resolver_mode,
            tool_id,
            args: &args_json,
            request_type: request_type.clone(),
            session_id: &record.handle.session_id,
            command_summary: Some(&normalized_summary),
        };
        let mut gate = self.new_gate(
            &record.handle.session_id,
            resolver_mode,
            record.session_policy.clone(),
            record.handle.workspace_root.to_string_lossy(),
        );
        let decision = gate.evaluate(request, tool.as_ref());
        Self::record_permission_decision_recorded_for_facade(
            record,
            tool_id,
            &resolver_mode,
            &request_type,
            &decision,
            gate.denial_count(),
        )?;
        match decision {
            PermissionResolution::Allow => Ok(FacadeToolMode::FastAutoApply),
            PermissionResolution::Deny { reason } => Ok(FacadeToolMode::Blocked(reason)),
            PermissionResolution::Ask { .. } => Ok(FacadeToolMode::RequirePermission(request_type)),
        }
    }

    pub(crate) fn persist_permission_decision_rule(
        &self,
        record: &mut RuntimeSessionRecord,
        request_type: Option<PermissionRequestType>,
        tool_id: &str,
        normalized_summary: &str,
        decision: &PermissionDecisionKind,
    ) -> Result<(), String> {
        let Some(request_type) = request_type else {
            return Ok(());
        };
        match decision {
            PermissionDecisionKind::AllowSession => {
                if let Some(rule) = permission_rule_from_decision(
                    format!(
                        "{}_{}_session",
                        record.handle.session_id,
                        tool_id.replace('.', "_")
                    ),
                    PermissionRuleScope::Session,
                    request_type,
                    tool_id,
                    normalized_summary,
                    decision.clone(),
                    "session approval from runtime facade",
                ) {
                    record.session_policy.add_or_replace(rule);
                }
            }
            PermissionDecisionKind::AllowProjectRule => {
                if let Some(rule) = permission_rule_from_decision(
                    format!(
                        "{}_project",
                        crate::patch::stable_text_hash(&format!("{tool_id}:{normalized_summary}"))
                    ),
                    PermissionRuleScope::Project,
                    request_type,
                    tool_id,
                    normalized_summary,
                    decision.clone(),
                    "project approval from runtime facade",
                ) {
                    self.rule_store.add_rule(rule)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn record_permission_decision_recorded_for_facade(
        record: &mut RuntimeSessionRecord,
        tool_id: &str,
        mode: &PermissionMode,
        request_type: &PermissionRequestType,
        decision: &PermissionResolution,
        denial_count_after: u32,
    ) -> Result<(), String> {
        record
            .session
            .record_runtime_event(
                "permission.decision.recorded",
                Actor::Runtime,
                serde_json::json!({
                    "tool_id": tool_id,
                    "mode": format!("{mode:?}"),
                    "request_type": format!("{request_type:?}"),
                    "decision": format!("{decision:?}"),
                    "denial_count_after": denial_count_after,
                })
                .to_string(),
            )
            .map_err(|error| format!("{error:?}"))
    }
}

fn permission_mode_for_facade_tool_mode(mode: &FacadeToolMode) -> PermissionMode {
    match mode {
        FacadeToolMode::FastAutoApply => PermissionMode::BypassPermissions,
        FacadeToolMode::Preview => PermissionMode::DontAsk,
        FacadeToolMode::RequirePermission(_) | FacadeToolMode::Blocked(_) => {
            PermissionMode::Default
        }
    }
}

fn facade_permission_tool_for_id(tool_id: &str) -> Box<dyn PermissionCheck> {
    match tool_id {
        "shell.command" => Box::new(ShellCommandTool),
        "patch.apply" => Box::new(PatchApplyTool),
        "file.write" => Box::new(FileWriteTool),
        "file.edit" | "file.multi_edit" => Box::new(FileEditTool),
        _ => Box::new(DefaultTool::new(tool_id)),
    }
}

fn facade_permission_args_json(args: &ToolExecutionArgs) -> serde_json::Value {
    let mut object = serde_json::Map::new();
    if let Some(value) = &args.path {
        object.insert("path".to_string(), serde_json::Value::String(value.clone()));
    }
    if let Some(value) = &args.command {
        object.insert(
            "command".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.content {
        object.insert(
            "content".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.old_string {
        object.insert(
            "old_string".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    if let Some(value) = &args.new_string {
        object.insert(
            "new_string".to_string(),
            serde_json::Value::String(value.clone()),
        );
    }
    serde_json::Value::Object(object)
}

pub(crate) fn permission_request_type_for_tool(tool_id: &str) -> Option<PermissionRequestType> {
    match tool_id {
        "shell.command" => Some(PermissionRequestType::Command),
        "file.write" | "file.edit" | "file.multi_edit" | "patch.apply" => {
            Some(PermissionRequestType::FileWrite)
        }
        "artifact.export" => Some(PermissionRequestType::ArtifactExport),
        _ => None,
    }
}

pub(crate) fn normalized_permission_summary(tool_id: &str, args: &ToolExecutionArgs) -> String {
    match tool_id {
        "shell.command" => format!("command: {}", args.command.as_deref().unwrap_or_default()),
        "patch.apply" | "file.edit" | "file.write" | "file.multi_edit" => format!(
            "{} path={} base_hash={}",
            tool_id,
            args.path.as_deref().unwrap_or_default(),
            args.base_hash.as_deref().unwrap_or_default()
        ),
        _ => format!("{tool_id}:{}", args.path.as_deref().unwrap_or_default()),
    }
}
