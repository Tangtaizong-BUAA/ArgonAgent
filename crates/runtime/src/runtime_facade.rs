//! GUI-ready runtime facade.
//!
//! TUI and future GUI clients should call this boundary instead of rebuilding
//! agent state, tool preview, permissions, or event export themselves.

use crate::agent_kernel::{AgentKernel, PermissionMode, TurnRouter};
use crate::approval_queue::{extract_approval_queue, ApprovalQueue};
use crate::error_recovery::ErrorRecoveryState;
use crate::event_log::EventLog;
use crate::hook_dispatcher::HookDispatcher;
use crate::live_http_transport::LiveHttpTransport;
use crate::native_agent_loop::{
    native_agent_tool_exposure_for_route, NativeAgentLoopResult, NativeAgentLoopStatus,
    NativeAgentLoopV2Request, NativeAgentToolExposure, PendingNativeToolExecution,
};
use crate::native_provider::NativeProviderEndpoint;
use crate::patch::{
    stable_text_hash, validate_patch_allowing_protected, PatchCheck, PatchValidation,
};
use crate::permission_policy::PermissionRuleSet;
use crate::runtime::context_service::{
    format_file_state_ranges, is_plateau_fallback_note, ContextService,
};
use crate::runtime::interrupt_service::InterruptService;
use crate::runtime::permission_service::{
    normalized_permission_summary, permission_request_type_for_tool, FacadeToolMode,
    PermissionService,
};
use crate::runtime::session_store::{RuntimeFileState, RuntimeSessionRecord, SessionStore};
use crate::runtime::subagent_store::SubagentStore;
use crate::session::AgentSession;
use crate::state::AgentState;
use crate::subagent::{
    validate_subagent_request, SubagentRequest, SubagentSession, SubagentStatus, SubagentSummary,
    SubagentType,
};
use crate::tool_execution::{
    execute_tool, ToolExecutionArgs, ToolExecutionMode, ToolExecutionRequest, ToolExecutionResult,
};
use crate::tool_result::{write_tool_result_artifact, ToolResultRecord};
use crate::ultra::{
    build_ultraplan_fixture, build_ultrareview_fixture, UltraPlanSpec, UltraReviewReport,
};
use researchcode_kernel::context::ContextBundle;
use researchcode_kernel::hooks::{Hook, HookDecision, HookDispatchPolicy, HookEvent};
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::{
    Actor, KernelEvent, PermissionDecisionKind, PermissionRequestType, PlanApprovalDecisionKind,
};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeModelMode {
    DeepSeek,
    Qwen,
}

impl RuntimeModelMode {
    pub fn family(&self) -> NativeModelFamily {
        match self {
            RuntimeModelMode::DeepSeek => NativeModelFamily::DeepSeek,
            RuntimeModelMode::Qwen => NativeModelFamily::Qwen,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RuntimeModelMode::DeepSeek => "deepseek",
            RuntimeModelMode::Qwen => "qwen",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutonomyMode {
    Conservative,
    FastAuto,
    ManualReview,
}

impl AutonomyMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            AutonomyMode::Conservative => "conservative",
            AutonomyMode::FastAuto => "fast_auto",
            AutonomyMode::ManualReview => "manual_review",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSessionHandle {
    pub project_id: String,
    pub session_id: String,
    pub task_id: String,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub model_mode: RuntimeModelMode,
    pub autonomy_mode: AutonomyMode,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeSessionSnapshot {
    pub session_id: String,
    pub state: AgentState,
    pub event_count: usize,
    pub model_mode: RuntimeModelMode,
    pub autonomy_mode: AutonomyMode,
    pub approval_queue: ApprovalQueue,
    pub pending_permission_count: usize,
    pub pending_plan_approval_count: usize,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub plan_mode_active: bool,
    pub session_memory_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAgentEventStream {
    pub session_id: String,
    pub jsonl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAgentEventDelta {
    pub session_id: String,
    pub from_cursor: usize,
    pub next_cursor: usize,
    pub events: Vec<String>,
    pub jsonl: String,
    pub has_more: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeContextRefPage {
    pub session_id: String,
    pub task_id: String,
    pub reference: String,
    pub event_id: String,
    pub sequence: u64,
    pub event_type: String,
    pub actor: Actor,
    pub payload_json: String,
    pub projected_message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePermissionDecisionOutcome {
    pub session_id: String,
    pub permission_id: String,
    pub tool_call_id: Option<String>,
    pub provider_tool_call_id: Option<String>,
    pub tool_id: Option<String>,
    pub resume_strategy: String,
    pub tool_executed: bool,
    pub model_continuation_required: bool,
    pub error_code: Option<String>,
    pub tool_result: Option<ToolExecutionResult>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimePendingNativeDecision {
    pub session_id: String,
    pub turn_id: Option<String>,
    pub permission_id: String,
    pub provider_tool_call_id: Option<String>,
    pub tool_call_id: String,
    pub tool_id: String,
    pub args: ToolExecutionArgs,
    pub blocked_event_jsonl: String,
    pub resume_strategy: String,
    pub created_timestamp: String,
    pub pending_tool: PendingNativeToolExecution,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FacadeToolOutcome {
    Executed(ToolExecutionResult),
    RequiresPlanApproval {
        plan_approval_id: String,
    },
    RequiresPermission {
        permission_id: String,
        request_type: PermissionRequestType,
    },
    BlockedByPolicy(String),
}

#[derive(Debug)]
pub struct RuntimeFacade {
    workspace_root: PathBuf,
    artifact_root: PathBuf,
    sessions: Arc<SessionStore>,
    subagents: Arc<SubagentStore>,
    context: Arc<ContextService>,
    permissions: Arc<PermissionService>,
    interrupt: Arc<InterruptService>,
}

#[path = "runtime_facade_impl.rs"]
mod runtime_facade_impl;
