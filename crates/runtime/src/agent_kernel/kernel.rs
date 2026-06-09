use super::{
    Compactor, ConvergenceEnforcer, EvidenceLedger, NativeLoopTurnController, PermissionGate,
    PermissionMode, ToolBatchGuardAction, ToolBatchSignatureStatus, TurnState,
};
use crate::context_budget::ContextBudget;
use crate::event_log::EventLog;
use crate::live_http_transport::LiveHttpTransport;
use crate::live_model_request::PreparedModelHttpRequest;
use crate::native_agent_loop::{
    run_native_agent_loop_v2_deepseek_with_kernel_and_interrupt, NativeAgentLoopResult,
    NativeAgentLoopV2Request, PendingNativeToolExecution,
};
use crate::native_profile::{profile_for_family, NativeProfile, NativeProfileInstance};
use crate::native_turn_controller::{
    evaluate_native_context_guard, NativeContextGuardReport, NativeTurnController,
};
use crate::permission_policy::{PermissionRuleSet, PermissionRuleStore};
use crate::session::AgentSession;
use crate::tcml::{PipelineOutcome, ToolCallPipeline};
use crate::tool_dispatcher::{
    schedule_tool_calls, ScheduledToolCall, ToolDispatchBatch, ToolDispatchError,
};
use crate::tool_execution::{
    execute_tool, json_string, ToolExecutionMode, ToolExecutionRequest, ToolExecutionResult,
};
use crate::tool_orchestration::{
    partition_tool_calls, ToolBatch, ToolCall as OrchestrationToolCall,
};
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::PermissionDecisionKind;
use std::path::Path;
use std::sync::{Arc, Mutex, RwLock};

/// Owns deterministic context projection decisions for native turns.
///
/// This first P3 slice makes the ownership boundary explicit. The remaining
/// context construction code is still called from the facade/native loop while
/// it is migrated behind this service.
#[derive(Debug, Default, Clone)]
pub struct ContextManager {
    pub openai_history_enabled: bool,
}

impl ContextManager {
    pub fn guard_prepared_request(
        &self,
        session: &mut AgentSession,
        family: &NativeModelFamily,
        context_budget: &ContextBudget,
        call_id: &str,
        stage: &str,
        prepared: &PreparedModelHttpRequest,
    ) -> Result<NativeContextGuardReport, String> {
        evaluate_native_context_guard(session, family, context_budget, call_id, stage, prepared)
    }
}

/// Owns model-facing TCML mediation policy.
///
/// The concrete mediation functions already live under `tcml`; this service is
/// the kernel-side authority marker used while call-sites are moved out of the
/// native loop.
#[derive(Debug, Default, Clone)]
pub struct TcmlService {
    pub emits_pipeline_completed: bool,
}

impl TcmlService {
    pub fn process_text(&self, raw: &str) -> PipelineOutcome {
        let mut pipeline = ToolCallPipeline::default();
        pipeline.process_text(raw)
    }
}

/// Owns native completion authority checks.
#[derive(Debug, Default, Clone)]
pub struct CompletionAuthority;

impl CompletionAuthority {
    pub fn ensure_can_complete(
        &self,
        turn_controller: &NativeTurnController,
        session: &mut AgentSession,
        reason: &str,
    ) -> Result<(), String> {
        turn_controller.ensure_can_complete(session, reason)
    }
}

/// Owns native tool batching, dispatch scheduling, and repeated-batch guard policy.
#[derive(Debug, Default, Clone)]
pub struct ToolOrchestrationService;

impl ToolOrchestrationService {
    pub fn partition_tool_batch(&self, calls: &[OrchestrationToolCall]) -> Vec<ToolBatch> {
        partition_tool_calls(calls)
    }

    pub fn schedule_dispatch(
        &self,
        calls: Vec<ScheduledToolCall>,
    ) -> Result<Vec<ToolDispatchBatch>, ToolDispatchError> {
        schedule_tool_calls(calls)
    }

    pub fn observe_batch_guard(
        &self,
        controller: &mut NativeLoopTurnController,
        session: &mut AgentSession,
        turn_state: &mut TurnState,
        iteration: usize,
        batch_signature: String,
        batch_status: ToolBatchSignatureStatus,
        repeated_cached_observation_batch: bool,
    ) -> Result<ToolBatchGuardAction, String> {
        controller.observe_tool_batch_guard(
            session,
            turn_state,
            iteration,
            batch_signature,
            batch_status,
            repeated_cached_observation_batch,
        )
    }
}

/// AgentKernel is the stable runtime entry point for a native agent turn.
///
/// P3 keeps the old native loop as the execution engine for now, but the kernel
/// now owns the service graph and exposes `run_turn` as the single facade-facing
/// entry point. Subsequent P3 slices should move the individual decisions behind
/// these fields instead of adding new inline logic to `native_agent_loop`.
#[derive(Debug, Clone)]
pub struct AgentKernel {
    request_scoped: bool,
    pub turn_controller: NativeLoopTurnController,
    pub compactor: Compactor,
    pub permission_gate: PermissionGate,
    pub context_manager: ContextManager,
    pub evidence_ledger: Arc<Mutex<EvidenceLedger>>,
    pub event_log_handle: Arc<RwLock<EventLog>>,
    pub convergence: ConvergenceEnforcer,
    pub tcml: TcmlService,
    pub completion: CompletionAuthority,
    pub tool_orchestration: ToolOrchestrationService,
    pub profile: NativeProfileInstance,
}

#[derive(Debug, Clone)]
pub struct PermissionResumeExecutionOutcome {
    pub tool_executed: bool,
    pub tool_result: Option<ToolExecutionResult>,
}

impl Default for AgentKernel {
    fn default() -> Self {
        Self {
            request_scoped: false,
            turn_controller: NativeLoopTurnController::new(),
            compactor: Compactor::default(),
            permission_gate: PermissionGate::new(
                Arc::new(PermissionRuleStore::new(
                    std::env::temp_dir()
                        .join("researchcode_agent_kernel_default_permission_policy.tsv"),
                )),
                PermissionRuleSet::default(),
                PermissionMode::Default,
                ".",
                "agent-kernel-default",
            ),
            context_manager: ContextManager {
                openai_history_enabled: true,
            },
            evidence_ledger: Arc::new(Mutex::new(EvidenceLedger::default())),
            event_log_handle: Arc::new(RwLock::new(EventLog::default())),
            convergence: ConvergenceEnforcer::default(),
            tcml: TcmlService {
                emits_pipeline_completed: true,
            },
            completion: CompletionAuthority,
            tool_orchestration: ToolOrchestrationService,
            profile: profile_for_family(NativeModelFamily::DeepSeek),
        }
    }
}

impl AgentKernel {
    pub fn for_request(request: &NativeAgentLoopV2Request) -> Self {
        let mut kernel = Self::default();
        kernel.request_scoped = true;
        kernel.profile = profile_for_family(request.endpoint.family.clone());
        kernel.permission_gate = PermissionGate::new(
            Arc::new(PermissionRuleStore::new(
                request.artifact_root.join("permission_policy.tsv"),
            )),
            PermissionRuleSet::default(),
            request.permission_mode,
            request.workspace_root.to_string_lossy(),
            request.session_id.clone(),
        );
        kernel
    }

    pub fn for_permission_resume(
        workspace_root: &Path,
        artifact_root: &Path,
        permission_mode: PermissionMode,
        session_id: &str,
        family: NativeModelFamily,
    ) -> Self {
        let mut kernel = Self::default();
        kernel.request_scoped = true;
        kernel.profile = profile_for_family(family);
        kernel.permission_gate = PermissionGate::new(
            Arc::new(PermissionRuleStore::new(
                artifact_root.join("permission_policy.tsv"),
            )),
            PermissionRuleSet::default(),
            permission_mode,
            workspace_root.to_string_lossy(),
            session_id.to_string(),
        );
        kernel
    }

    pub fn classify_turn(
        &self,
        prompt: &str,
        history_hint: Option<&str>,
        turn_index: u32,
    ) -> super::TurnRoute {
        let _ = self;
        super::TurnRouter::classify(prompt, history_hint, turn_index)
    }

    pub fn run_turn<T: LiveHttpTransport>(
        &self,
        transport: &T,
        request: NativeAgentLoopV2Request,
        event_sink: Option<&mut dyn FnMut(&str)>,
    ) -> Result<NativeAgentLoopResult, String> {
        let interrupt = std::sync::atomic::AtomicBool::new(false);
        self.run_turn_with_interrupt(transport, request, event_sink, &interrupt)
    }

    pub fn run_turn_with_interrupt<T: LiveHttpTransport>(
        &self,
        transport: &T,
        request: NativeAgentLoopV2Request,
        event_sink: Option<&mut dyn FnMut(&str)>,
        interrupt: &std::sync::atomic::AtomicBool,
    ) -> Result<NativeAgentLoopResult, String> {
        if !self.request_scoped {
            return Err(
                "AgentKernel::run_turn requires request-scoped services; construct it with AgentKernel::for_request".to_string(),
            );
        }
        self.validate_request_profile(&request)?;
        match event_sink {
            Some(sink) => run_native_agent_loop_v2_deepseek_with_kernel_and_interrupt(
                transport,
                request,
                self.clone(),
                Some(sink),
                interrupt,
            ),
            None => run_native_agent_loop_v2_deepseek_with_kernel_and_interrupt(
                transport,
                request,
                self.clone(),
                None,
                interrupt,
            ),
        }
    }

    pub fn resume_pending_tool_after_permission_decision(
        &self,
        workspace_root: &Path,
        pending: &PendingNativeToolExecution,
        decision: PermissionDecisionKind,
    ) -> Result<PermissionResumeExecutionOutcome, String> {
        if !self.request_scoped {
            return Err(
                "AgentKernel::resume_pending_tool_after_permission_decision requires request-scoped services".to_string(),
            );
        }
        if !matches!(
            decision,
            PermissionDecisionKind::AllowOnce
                | PermissionDecisionKind::AllowSession
                | PermissionDecisionKind::AllowProjectRule
        ) {
            return Ok(PermissionResumeExecutionOutcome {
                tool_executed: false,
                tool_result: None,
            });
        }

        let result = execute_tool(&ToolExecutionRequest {
            workspace_root: workspace_root.to_path_buf(),
            tool_call_id: pending.tool_call_id.clone(),
            tool_id: pending.tool_id.clone(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(decision),
            },
            args: pending.args.clone(),
        })
        .unwrap_or_else(|error| ToolExecutionResult {
            tool_call_id: pending.tool_call_id.clone(),
            tool_id: pending.tool_id.clone(),
            ok: false,
            preview: format!("tool execution failed during permission resume: {error:?}"),
            detail_json: format!(
                "{{\"ok\":false,\"error_code\":\"permission_resume_tool_failed\",\"raw_error\":{},\"recoverable\":true}}",
                json_string(&format!("{error:?}"))
            ),
            exit_code: None,
        });

        Ok(PermissionResumeExecutionOutcome {
            tool_executed: true,
            tool_result: Some(result),
        })
    }

    fn validate_request_profile(&self, request: &NativeAgentLoopV2Request) -> Result<(), String> {
        if self.profile.family() != request.endpoint.family {
            return Err(format!(
                "AgentKernel profile {} does not match request family {:?}",
                self.profile.profile_name(),
                request.endpoint.family
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_kernel::PermissionMode;
    use crate::live_http_transport::{LiveHttpResponse, ScriptedLiveHttpTransport};
    use crate::native_agent_loop::{
        NativeAgentLoopStatus, NativeAgentPermissionDecision, NativeAgentToolExposure,
    };
    use crate::native_provider::NativeProviderEndpoint;
    use crate::permission_policy::{PermissionRuleSet, PermissionRuleStore};
    use crate::state::AgentState;
    use researchcode_kernel::PermissionDecisionKind;
    use std::fs;
    use std::sync::Arc;

    #[test]
    fn kernel_owns_native_turn_services() {
        let kernel = AgentKernel::default();
        assert!(kernel.context_manager.openai_history_enabled);
        assert_eq!(kernel.permission_gate.denial_count(), 0);
        assert!(kernel.event_log_handle.read().unwrap().is_empty());
        assert!(kernel.tcml.emits_pipeline_completed);
        assert_eq!(
            kernel.turn_controller.max_loop_guard_recoveries(),
            NativeLoopTurnController::new().max_loop_guard_recoveries()
        );
    }

    #[test]
    fn kernel_for_request_materializes_request_scoped_services() {
        let temp = tempfile::tempdir().unwrap();
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        let request = NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: temp.path().join("workspace"),
            artifact_root: temp.path().join("artifacts"),
            endpoint: crate::native_provider::NativeProviderEndpoint::qwen36_27b_custom_endpoint(),
            prompt: "hello".to_string(),
            max_tokens: 128,
            max_iterations: 1,
            max_tool_calls: 1,
            tool_exposure: crate::native_agent_loop::NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Plan,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        let kernel = AgentKernel::for_request(&request);
        assert_eq!(kernel.profile.family(), NativeModelFamily::Qwen);
        assert_eq!(kernel.permission_gate.denial_count(), 0);
        assert!(kernel.validate_request_profile(&request).is_ok());
    }

    #[test]
    fn kernel_clones_share_evidence_ledger_handle() {
        let kernel = AgentKernel::default();
        let cloned = kernel.clone();
        kernel.evidence_ledger.lock().unwrap().push_legacy((
            "provider_tool_1".to_string(),
            "file.read".to_string(),
            "{\"path\":\"README.md\"}".to_string(),
            crate::tool_execution::ToolExecutionResult {
                tool_call_id: "tool_1".to_string(),
                tool_id: "file.read".to_string(),
                ok: true,
                preview: "read README".to_string(),
                detail_json: "{}".to_string(),
                exit_code: None,
            },
        ));
        assert_eq!(cloned.evidence_ledger.lock().unwrap().len(), 1);
    }

    #[test]
    fn kernel_run_turn_rejects_unscoped_default_kernel() {
        let temp = tempfile::tempdir().unwrap();
        let request = NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: temp.path().join("workspace"),
            artifact_root: temp.path().join("artifacts"),
            endpoint: crate::native_provider::NativeProviderEndpoint::deepseek_v4_flash_openai(),
            prompt: "hello".to_string(),
            max_tokens: 128,
            max_iterations: 1,
            max_tool_calls: 1,
            tool_exposure: crate::native_agent_loop::NativeAgentToolExposure::ReadOnly,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(Vec::new());
        let error = AgentKernel::default()
            .run_turn(&transport, request, None)
            .unwrap_err();
        assert!(error.contains("AgentKernel::for_request"));
    }

    #[test]
    fn kernel_run_turn_uses_caller_owned_permission_gate() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let artifacts = temp.path().join("artifacts");
        fs::create_dir_all(&workspace).unwrap();
        let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_kernel_owned_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"kernel-owned.txt\",\"content\":\"owned by kernel\n\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Created kernel-owned.txt."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        let request = NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_kernel_owned_services".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: workspace.clone(),
            artifact_root: artifacts.clone(),
            endpoint,
            prompt: "Create kernel-owned.txt.".to_string(),
            max_tokens: 4096,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        let mut kernel = AgentKernel::for_request(&request);
        kernel.permission_gate = PermissionGate::new(
            Arc::new(PermissionRuleStore::new(
                artifacts.join("kernel_policy.tsv"),
            )),
            PermissionRuleSet::default(),
            PermissionMode::BypassPermissions,
            workspace.to_string_lossy(),
            request.session_id.clone(),
        );

        let result = kernel.run_turn(&transport, request, None).unwrap();
        let written_path = workspace.join("kernel-owned.txt");
        assert!(
            written_path.exists(),
            "expected caller-owned permission gate to execute write; result={result:?}; events={}",
            result.event_jsonl
        );
        let written = fs::read_to_string(written_path).unwrap();

        assert_eq!(
            result.status,
            NativeAgentLoopStatus::Completed,
            "events={}",
            result.event_jsonl
        );
        assert_eq!(
            result.final_state,
            AgentState::Completed,
            "events={}",
            result.event_jsonl
        );
        assert_eq!(written, "owned by kernel\n");
        assert!(result.event_jsonl.contains("\"tool_id\":\"file.write\""));
        assert!(!result
            .event_jsonl
            .contains("\"event_type\":\"permission.requested\""));
    }

    #[test]
    fn kernel_run_turn_uses_caller_owned_permission_gate_for_shell_command() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let artifacts = temp.path().join("artifacts");
        fs::create_dir_all(&workspace).unwrap();
        let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_kernel_owned_shell","name":"shell_command","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"find . -maxdepth 0\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Ran find."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        let request = NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_kernel_owned_shell".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: workspace.clone(),
            artifact_root: artifacts.clone(),
            endpoint,
            prompt: "Run a harmless shell command.".to_string(),
            max_tokens: 4096,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        let mut kernel = AgentKernel::for_request(&request);
        kernel.permission_gate = PermissionGate::new(
            Arc::new(PermissionRuleStore::new(
                artifacts.join("kernel_shell_policy.tsv"),
            )),
            PermissionRuleSet::default(),
            PermissionMode::BypassPermissions,
            workspace.to_string_lossy(),
            request.session_id.clone(),
        );

        let result = kernel.run_turn(&transport, request, None).unwrap();

        assert_eq!(
            result.status,
            NativeAgentLoopStatus::Completed,
            "events={}",
            result.event_jsonl
        );
        assert_eq!(
            result.final_state,
            AgentState::Completed,
            "events={}",
            result.event_jsonl
        );
        assert!(result.event_jsonl.contains("\"tool_id\":\"shell.command\""));
        assert!(result.event_jsonl.contains("find . -maxdepth 0"));
        assert!(!result
            .event_jsonl
            .contains("\"event_type\":\"permission.requested\""));
    }

    #[test]
    fn kernel_run_turn_preserves_provided_shell_permission_decision() {
        let temp = tempfile::tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        let artifacts = temp.path().join("artifacts");
        fs::create_dir_all(&workspace).unwrap();
        let transport = ScriptedLiveHttpTransport::new(vec![
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_kernel_provided_shell","name":"shell_command","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"command\":\"printf provided-shell\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Ran approved command."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.live_calls_enabled_by_default = true;
        let request = NativeAgentLoopV2Request {
            project_id: "proj".to_string(),
            session_id: "sess_kernel_provided_shell".to_string(),
            task_id: "task".to_string(),
            turn_id: None,
            workspace_root: workspace,
            artifact_root: artifacts,
            endpoint,
            prompt: "Run an approved shell command.".to_string(),
            max_tokens: 4096,
            max_iterations: 4,
            max_tool_calls: 8,
            tool_exposure: NativeAgentToolExposure::FastAutoWrite,
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: vec![NativeAgentPermissionDecision {
                permission_id: "native_loop_v2_command_perm_0".to_string(),
                decision: PermissionDecisionKind::AllowOnce,
            }],
            deepseek_adaptation: None,
            error_recovery: None,
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        let kernel = AgentKernel::for_request(&request);

        let result = kernel.run_turn(&transport, request, None).unwrap();

        assert_eq!(result.status, NativeAgentLoopStatus::Completed);
        assert_eq!(result.final_state, AgentState::Completed);
        assert!(result.event_jsonl.contains("\"tool_id\":\"shell.command\""));
        assert!(result.event_jsonl.contains("provided-shell"));
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"permission.requested\""));
        assert!(result
            .event_jsonl
            .contains("\"event_type\":\"permission.decided\""));
        assert!(result.pending_tool.is_none());
    }
}
