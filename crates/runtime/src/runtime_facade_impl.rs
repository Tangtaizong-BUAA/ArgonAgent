use super::*;
use crate::agent_kernel::permission_gate::{classify_command_with_reasons, CommandDecision};

impl RuntimeFacade {
    /// Compatibility boundary for Ctrl-C aware V2 live CLI runs.
    ///
    /// This keeps the CLI from importing native-loop entrypoints directly
    /// while the interrupt flag is moved into the facade session API.
    pub fn run_deepseek_agent_loop_request_with_interrupt<T: LiveHttpTransport>(
        transport: &T,
        request: NativeAgentLoopV2Request,
        event_sink: Option<&mut dyn FnMut(&str)>,
        interrupt: &std::sync::atomic::AtomicBool,
    ) -> Result<NativeAgentLoopResult, String> {
        InterruptService::run_deepseek_agent_loop_request_with_interrupt(
            transport, request, event_sink, interrupt,
        )
    }

    pub fn new(workspace_root: impl Into<PathBuf>, artifact_root: impl Into<PathBuf>) -> Self {
        let artifact_root = artifact_root.into();
        Self {
            workspace_root: workspace_root.into(),
            permissions: Arc::new(PermissionService::new(
                artifact_root.join("permission_policy.tsv"),
            )),
            artifact_root,
            sessions: Arc::new(SessionStore::new()),
            subagents: Arc::new(SubagentStore::new()),
            context: Arc::new(ContextService::new()),
            interrupt: Arc::new(InterruptService::new()),
        }
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn interrupt_handle(&self) -> Arc<std::sync::atomic::AtomicBool> {
        self.interrupt.handle()
    }

    pub fn interrupt(&self) {
        self.interrupt.interrupt();
    }

    pub fn reset_interrupt(&self) {
        self.interrupt.reset();
    }

    pub fn is_interrupted(&self) -> bool {
        self.interrupt.is_interrupted()
    }

    pub fn cancel_session(&self, session_id: &str) -> Result<(), String> {
        self.interrupt();
        self.sessions
            .with_mut(session_id, |record| {
                let state = record.session.state();
                record
                    .session
                    .record_runtime_event(
                        "runtime.turn_cancel_requested",
                        Actor::Runtime,
                        format!("{{\"session_id\":{}}}", json_string(session_id)),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                record.pending_native_decision = None;
                record
                    .session
                    .clear_pending_runtime_decisions("cancel_session")
                    .map_err(|error| format!("{error:?}"))?;
                if matches!(
                    state,
                    AgentState::Completed | AgentState::Failed | AgentState::Cancelled
                ) {
                    return Ok(());
                }
                record
                    .session
                    .transition_to(AgentState::Cancelled)
                    .map_err(|error| format!("{error:?}"))
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))?
    }

    pub fn set_autonomy_mode(
        &self,
        session_id: &str,
        autonomy_mode: AutonomyMode,
    ) -> Result<RuntimeSessionHandle, String> {
        self.sessions
            .with_mut(session_id, |record| {
                record.handle.autonomy_mode = autonomy_mode;
                record
                    .session
                    .record_runtime_event(
                        "session.autonomy_mode_changed",
                        Actor::Runtime,
                        format!(
                            "{{\"session_id\":{},\"autonomy_mode\":{}}}",
                            json_string(session_id),
                            json_string(record.handle.autonomy_mode.as_str())
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                Ok(record.handle.clone())
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))?
    }

    pub fn start_session(
        &self,
        workspace: Option<PathBuf>,
        model_mode: RuntimeModelMode,
        autonomy_mode: AutonomyMode,
    ) -> Result<RuntimeSessionHandle, String> {
        let nonce = monotonic_nonce()?;
        let session_id = format!("runtime_session_{nonce}");
        let task_id = format!("runtime_task_{nonce}");
        let mut session = AgentSession::new("local", session_id.clone(), task_id.clone())
            .map_err(|error| format!("{error:?}"))?;
        session
            .transition_to(AgentState::Planning)
            .and_then(|_| session.transition_to(AgentState::RetrievingContext))
            .and_then(|_| session.transition_to(AgentState::Executing))
            .map_err(|error| format!("{error:?}"))?;
        let handle = RuntimeSessionHandle {
            project_id: "local".to_string(),
            session_id: session_id.clone(),
            task_id,
            workspace_root: workspace.unwrap_or_else(|| self.workspace_root.clone()),
            artifact_root: self.artifact_root.join(&session_id),
            model_mode,
            autonomy_mode,
        };
        let record = RuntimeSessionRecord {
            handle: handle.clone(),
            session,
            session_policy: PermissionRuleSet::default(),
            session_memory: Vec::new(),
            file_state: HashMap::new(),
            plan_mode_active: false,
            repeated_tool_failures: HashMap::new(),
            path_corrections: HashMap::new(),
            discovered_roots: Vec::new(),
            native_tool_completion: HashMap::new(),
            error_recovery: ErrorRecoveryState::default(),
            pending_native_decision: None,
        };
        self.sessions.insert(session_id, record);
        Ok(handle)
    }

    pub fn submit_user_message(&self, session_id: &str, text: &str) -> Result<(), String> {
        if text.trim().is_empty() {
            return Err("user message cannot be empty".to_string());
        }
        self.sessions
            .with_mut(session_id, |record| {
                if matches!(
                    record.session.state(),
                    AgentState::Completed | AgentState::Failed | AgentState::Cancelled
                ) {
                    let turn_id = format!("runtime_user_turn_{}", monotonic_nonce()?);
                    record
                        .session
                        .begin_interactive_turn(&turn_id, "submit_user_message")
                        .map_err(|error| format!("{error:?}"))?;
                }
                let stream_id = format!("{}_user_input", session_id);
                record
                    .session
                    .record_model_stream_delta(&stream_id, "user", "input", text)
                    .map_err(|error| format!("{error:?}"))?;
                remember_session(record, format!("user: {}", trim_for_memory(text, 320)));
                Ok(())
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))?
    }

    pub fn stream_agent_events(&self, session_id: &str) -> Result<RuntimeAgentEventStream, String> {
        self.sessions
            .with_ref(session_id, |record| RuntimeAgentEventStream {
                session_id: session_id.to_string(),
                jsonl: record.session.export_events_jsonl(),
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))
    }

    pub fn stream_agent_events_since(
        &self,
        session_id: &str,
        cursor: usize,
        max_events: Option<usize>,
    ) -> Result<RuntimeAgentEventDelta, String> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        let all_jsonl = record.session.export_events_jsonl();
        let lines = all_jsonl.lines().collect::<Vec<_>>();
        let cursor = cursor.min(lines.len());
        let limit = max_events.unwrap_or(usize::MAX);
        let end = cursor.saturating_add(limit).min(lines.len());
        let events = lines[cursor..end]
            .iter()
            .map(|line| (*line).to_string())
            .collect::<Vec<_>>();
        let jsonl = events.join("\n");
        Ok(RuntimeAgentEventDelta {
            session_id: session_id.to_string(),
            from_cursor: cursor,
            next_cursor: end,
            events,
            jsonl: if jsonl.is_empty() {
                String::new()
            } else {
                format!("{jsonl}\n")
            },
            has_more: end < lines.len(),
        })
    }

    pub fn page_context_ref(
        &self,
        session_id: &str,
        reference: &str,
    ) -> Result<RuntimeContextRefPage, String> {
        if !reference.starts_with("ref://event/") {
            return Err(format!(
                "unsupported context reference {reference}; expected ref://event/<sequence>"
            ));
        }
        self.sessions
            .with_ref(session_id, |record| {
                let page = record
                    .session
                    .event_log()
                    .page_ref(reference)
                    .ok_or_else(|| format!("unknown context reference {reference}"))?;
                if page.event.session_id.as_deref() != Some(session_id) {
                    return Err(format!(
                        "context reference {reference} belongs to session {:?}, not {session_id}",
                        page.event.session_id
                    ));
                }
                if page.event.task_id.as_deref() != Some(record.handle.task_id.as_str()) {
                    return Err(format!(
                        "context reference {reference} belongs to task {:?}, not {}",
                        page.event.task_id, record.handle.task_id
                    ));
                }
                Ok(RuntimeContextRefPage {
                    session_id: session_id.to_string(),
                    task_id: record.handle.task_id.clone(),
                    reference: page.reference,
                    event_id: page.event.event_id,
                    sequence: page.event.sequence,
                    event_type: page.event.event_type,
                    actor: page.event.actor,
                    payload_json: page.event.payload_json,
                    projected_message: page.projected_message,
                })
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))?
    }

    pub fn ingest_agent_event_jsonl_line(
        &self,
        session_id: &str,
        line: &str,
    ) -> Result<(), String> {
        if line.trim().is_empty() {
            return Ok(());
        }
        let event = EventLog::parse_jsonl_event(line).map_err(|error| format!("{error:?}"))?;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        record.session.merge_events(&[event]);
        Ok(())
    }

    fn ingest_native_loop_event_jsonl_line(
        &self,
        session_id: &str,
        line: &str,
        id_suffix_seq: u64,
    ) -> Result<bool, String> {
        if line.trim().is_empty() {
            return Ok(false);
        }
        let event = EventLog::parse_jsonl_event(line).map_err(|error| format!("{error:?}"))?;
        if !native_loop_event_visible_to_facade(&event) {
            return Ok(false);
        }
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        record
            .session
            .merge_events_with_id_suffix(&[event.clone()], id_suffix_seq);
        apply_native_loop_event_side_effect(record, &event);
        Ok(true)
    }

    pub fn submit_permission_decision(
        &self,
        session_id: &str,
        permission_id: &str,
        decision: PermissionDecisionKind,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        // Validate the submitted ID matches the session's current pending permission.
        match record.session.pending_permission_id() {
            Some(pending_id) if pending_id == permission_id => {}
            Some(mismatched) => {
                return Err(format!(
                    "permission_id mismatch: submitted {permission_id} but session has pending {mismatched}"
                ));
            }
            None => {
                return Err(format!("no pending permission in session {session_id}"));
            }
        }
        record
            .session
            .decide_permission(decision)
            .map_err(|error| format!("{error:?}"))
    }

    pub fn submit_permission_decision_with_outcome(
        &self,
        session_id: &str,
        permission_id: &str,
        decision: PermissionDecisionKind,
    ) -> Result<RuntimePermissionDecisionOutcome, String> {
        if self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(session_id)
            .and_then(|record| record.pending_native_decision.as_ref())
            .is_some_and(|pending| pending.permission_id == permission_id)
        {
            return self.resume_native_loop_after_permission_decision(
                session_id,
                permission_id,
                decision,
            );
        }
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        match record.session.pending_permission_id() {
            Some(pending_id) if pending_id == permission_id => {}
            Some(mismatched) => {
                return Err(format!(
                    "permission_id mismatch: submitted {permission_id} but session has pending {mismatched}"
                ));
            }
            None => {
                return Err(format!("no pending permission in session {session_id}"));
            }
        }

        let (tool_call_id, provider_tool_call_id, tool_id) =
            infer_permission_tool_hint(record, permission_id);
        record
            .session
            .decide_permission(decision.clone())
            .map_err(|error| format!("{error:?}"))?;

        Ok(RuntimePermissionDecisionOutcome {
            session_id: session_id.to_string(),
            permission_id: permission_id.to_string(),
            tool_call_id,
            provider_tool_call_id,
            tool_id,
            resume_strategy: "decision_recorded".to_string(),
            tool_executed: false,
            model_continuation_required: false,
            error_code: None,
            tool_result: None,
        })
    }

    pub fn resume_native_loop_after_permission_decision(
        &self,
        session_id: &str,
        permission_id: &str,
        decision: PermissionDecisionKind,
    ) -> Result<RuntimePermissionDecisionOutcome, String> {
        let (pending, workspace_root, artifact_root, model_family, should_execute) = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let record = sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("unknown session_id {session_id}"))?;
            let pending = record
                .pending_native_decision
                .clone()
                .ok_or_else(|| format!("no pending native decision in session {session_id}"))?;
            let current_turn_id = record.session.current_turn_id().map(str::to_string);
            if pending.turn_id != current_turn_id {
                record.pending_native_decision = None;
                record
                    .session
                    .clear_pending_runtime_decisions("stale_permission_decision")
                    .map_err(|error| format!("{error:?}"))?;
                return Err(format!(
                    "stale permission decision: submitted {permission_id} for turn {:?} but current turn is {:?}",
                    pending.turn_id, current_turn_id
                ));
            }
            if pending.permission_id != permission_id {
                return Err(format!(
                    "permission_id mismatch: submitted {permission_id} but session has pending {}",
                    pending.permission_id
                ));
            }
            match record.session.pending_permission_id() {
                Some(pending_id) if pending_id == permission_id => {}
                Some(mismatched) => {
                    return Err(format!(
                        "permission_id mismatch: submitted {permission_id} but session has pending {mismatched}"
                    ));
                }
                None => {
                    return Err(format!("no pending permission in session {session_id}"));
                }
            }
            record
                .session
                .record_runtime_event(
                    "runtime.permission_resume.started",
                    Actor::Runtime,
                    format!(
                        "{{\"session_id\":{},\"permission_id\":{},\"tool_call_id\":{},\"provider_tool_call_id\":{},\"tool_id\":{},\"resume_strategy\":{}}}",
                        json_string(session_id),
                        json_string(permission_id),
                        json_string(&pending.tool_call_id),
                        opt_json_string(pending.provider_tool_call_id.as_deref()),
                        json_string(&pending.tool_id),
                        json_string(&pending.resume_strategy)
                    ),
                )
                .and_then(|_| record.session.decide_permission(decision.clone()))
                .map_err(|error| format!("{error:?}"))?;
            let should_execute = matches!(
                decision,
                PermissionDecisionKind::AllowOnce
                    | PermissionDecisionKind::AllowSession
                    | PermissionDecisionKind::AllowProjectRule
            );
            if !should_execute {
                record.pending_native_decision = None;
                record
                    .session
                    .record_runtime_event(
                        "runtime.permission_resume.completed",
                        Actor::Runtime,
                        format!(
                            "{{\"session_id\":{},\"permission_id\":{},\"tool_call_id\":{},\"tool_id\":{},\"tool_executed\":false,\"model_continuation_required\":false,\"decision\":\"denied\"}}",
                            json_string(session_id),
                            json_string(permission_id),
                            json_string(&pending.tool_call_id),
                            json_string(&pending.tool_id)
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                return Ok(RuntimePermissionDecisionOutcome {
                    session_id: session_id.to_string(),
                    permission_id: permission_id.to_string(),
                    tool_call_id: Some(pending.tool_call_id),
                    provider_tool_call_id: pending.provider_tool_call_id,
                    tool_id: Some(pending.tool_id),
                    resume_strategy: "pending_native_decision".to_string(),
                    tool_executed: false,
                    model_continuation_required: false,
                    error_code: Some("permission_denied".to_string()),
                    tool_result: None,
                });
            }
            (
                pending,
                record.handle.workspace_root.clone(),
                record.handle.artifact_root.clone(),
                record.handle.model_mode.family(),
                should_execute,
            )
        };

        debug_assert!(should_execute);
        let kernel = AgentKernel::for_permission_resume(
            &workspace_root,
            &artifact_root,
            PermissionMode::Default,
            session_id,
            model_family,
        );
        let resume_execution = kernel.resume_pending_tool_after_permission_decision(
            &workspace_root,
            &pending.pending_tool,
            decision.clone(),
        )?;
        let result = resume_execution.tool_result.ok_or_else(|| {
            "permission resume returned no tool result after allow decision".to_string()
        })?;

        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        if pending.tool_id == "patch.apply" && result.ok {
            if let (Some(patch_id), Some(path)) = (
                pending.pending_tool.patch_id.clone(),
                pending.args.path.clone(),
            ) {
                record
                    .session
                    .record_patch_applied(patch_id, path)
                    .map_err(|error| format!("{error:?}"))?;
            }
        }
        record
            .session
            .record_tool_call_completed_with_provider_id(
                &pending.tool_call_id,
                pending.provider_tool_call_id.clone(),
                &pending.tool_id,
                result.ok,
            )
            .map_err(|error| format!("{error:?}"))?;
        let artifact_store = crate::artifact::ArtifactStore::new(&artifact_root);
        let artifact = write_tool_result_artifact(
            &artifact_store,
            &format!(
                "runtime_permission_resume_tool_result_{}",
                pending.tool_call_id
            ),
            &ToolResultRecord::new(
                &pending.tool_call_id,
                &pending.tool_id,
                result.ok,
                result.preview.clone(),
                result.detail_json.clone(),
            ),
        )
        .map_err(|error| error.to_string())?;
        record
            .session
            .record_tool_result_artifact_with_provider_id(
                &pending.tool_call_id,
                pending.provider_tool_call_id.clone(),
                &pending.tool_id,
                artifact.artifact_id,
                artifact.content_hash,
                result.preview.clone(),
            )
            .and_then(|_| {
                record.session.record_runtime_event(
                    "runtime.permission_resume.tool_executed",
                    Actor::Runtime,
                    format!(
                        "{{\"session_id\":{},\"permission_id\":{},\"tool_call_id\":{},\"provider_tool_call_id\":{},\"tool_id\":{},\"ok\":{},\"preview\":{}}}",
                        json_string(session_id),
                        json_string(permission_id),
                        json_string(&pending.tool_call_id),
                        opt_json_string(pending.provider_tool_call_id.as_deref()),
                        json_string(&pending.tool_id),
                        result.ok,
                        json_string(&result.preview)
                    ),
                )
            })
            .and_then(|_| {
                record.session.record_runtime_event(
                    "runtime.permission_resume.completed",
                    Actor::Runtime,
                    format!(
                        "{{\"session_id\":{},\"permission_id\":{},\"tool_call_id\":{},\"provider_tool_call_id\":{},\"tool_id\":{},\"tool_executed\":true,\"model_continuation_required\":{},\"ok\":{}}}",
                        json_string(session_id),
                        json_string(permission_id),
                        json_string(&pending.tool_call_id),
                        opt_json_string(pending.provider_tool_call_id.as_deref()),
                        json_string(&pending.tool_id),
                        result.ok,
                        result.ok
                    ),
                )
            })
            .map_err(|error| format!("{error:?}"))?;
        update_runtime_tool_state(record, &result);
        if matches!(
            record.session.state(),
            AgentState::ApplyingPatch | AgentState::RunningCommand
        ) {
            if result.ok {
                record
                    .session
                    .transition_to(AgentState::Executing)
                    .map_err(|error| format!("{error:?}"))?;
            } else {
                record
                    .session
                    .transition_to(AgentState::DiagnosingFailure)
                    .map_err(|error| format!("{error:?}"))?;
            }
        }
        record.pending_native_decision = None;
        Ok(RuntimePermissionDecisionOutcome {
            session_id: session_id.to_string(),
            permission_id: permission_id.to_string(),
            tool_call_id: Some(pending.tool_call_id),
            provider_tool_call_id: pending.provider_tool_call_id,
            tool_id: Some(pending.tool_id),
            resume_strategy: "pending_native_decision".to_string(),
            tool_executed: true,
            model_continuation_required: result.ok,
            error_code: (!result.ok).then(|| "permission_resume_tool_failed".to_string()),
            tool_result: Some(result),
        })
    }

    pub fn submit_plan_decision(
        &self,
        session_id: &str,
        plan_id: &str,
        decision: PlanApprovalDecisionKind,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        // Validate the submitted ID matches the session's current pending plan.
        match record.session.pending_plan_approval_id() {
            Some(pending_id) if pending_id == plan_id => {}
            Some(mismatched) => {
                return Err(format!(
                    "plan_id mismatch: submitted {plan_id} but session has pending {mismatched}"
                ));
            }
            None => {
                return Err(format!("no pending plan approval in session {session_id}"));
            }
        }
        let approved = matches!(decision, PlanApprovalDecisionKind::Approve);
        if approved {
            record.plan_mode_active = false;
        }
        let decision_label = match &decision {
            PlanApprovalDecisionKind::Approve => "approve",
            PlanApprovalDecisionKind::Reject => "reject",
            PlanApprovalDecisionKind::RequestRevision => "request_revision",
        };
        record
            .session
            .decide_plan(decision)
            .map_err(|error| format!("{error:?}"))?;
        let plan_tool_call_id = plan_id
            .strip_suffix("_plan_approval")
            .unwrap_or(plan_id)
            .to_string();
        let provider_tool_call_id =
            infer_provider_tool_call_id_from_session(&record.session, &plan_tool_call_id);
        let plan_result = format!("PlanApproval: {decision_label}");
        record
            .session
            .record_tool_call_completed_with_provider_id(
                &plan_tool_call_id,
                provider_tool_call_id.clone(),
                "plan.enter",
                true,
            )
            .and_then(|_| {
                record.session.record_tool_result_artifact_with_provider_id(
                    &plan_tool_call_id,
                    provider_tool_call_id,
                    "plan.enter",
                    format!("artifact_{plan_tool_call_id}"),
                    stable_text_hash(&plan_result),
                    plan_result,
                )
            })
            .map_err(|error| format!("{error:?}"))?;
        if approved {
            remember_session(
                record,
                format!(
                    "plan {plan_id} approved; continue from existing evidence and do not reread covered plan/file ranges"
                ),
            );
            record
                .session
                .record_runtime_event(
                    "runtime.plan_approval.model_continued",
                    Actor::Runtime,
                    format!(
                        "{{\"session_id\":{},\"plan_id\":{},\"strategy\":\"next_native_turn_with_evidence_ledger\",\"next_action_hint\":\"Continue implementation from the approved plan and existing evidence; do not reread covered plan/file ranges.\"}}",
                        json_string(session_id),
                        json_string(plan_id)
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
        }
        Ok(())
    }

    pub fn preview_tool(
        &self,
        workspace_root: &Path,
        tool_call_id: &str,
        tool_id: &str,
        args: ToolExecutionArgs,
    ) -> Result<ToolExecutionResult, String> {
        execute_tool(&ToolExecutionRequest {
            workspace_root: workspace_root.to_path_buf(),
            tool_call_id: tool_call_id.to_string(),
            tool_id: tool_id.to_string(),
            mode: ToolExecutionMode::ReadOnlyPreview,
            args,
        })
        .map_err(|error| format!("{error:?}"))
    }

    pub fn execute_subagent_tool(
        &self,
        subagent_id: &str,
        tool_call_id: &str,
        tool_id: &str,
        args: ToolExecutionArgs,
        permission_decision: Option<PermissionDecisionKind>,
    ) -> Result<ToolExecutionResult, String> {
        let subagent = {
            let subagents = self
                .subagents
                .subagents_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            subagents
                .get(subagent_id)
                .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?
                .clone()
        };
        if subagent.status == SubagentStatus::Cancelled {
            return Err(format!("subagent {subagent_id} cancelled"));
        }
        if !subagent.tool_allowlist.iter().any(|item| item == tool_id) {
            return Err(format!(
                "subagent {subagent_id} is not allowed to use {tool_id}"
            ));
        }
        if subagent_tool_writes(tool_id) {
            let path = args
                .path
                .as_deref()
                .ok_or_else(|| format!("{tool_id} requires path for write_scope enforcement"))?;
            if !path_is_within_any_scope(path, &subagent.write_scope) {
                self.record_subagent_tool_blocked(
                    subagent_id,
                    tool_call_id,
                    tool_id,
                    &format!("path {path} is outside subagent write_scope"),
                )?;
                return Err(format!(
                    "subagent {subagent_id} blocked {tool_id}: path {path} is outside write_scope"
                ));
            }
        }
        let workspace_root = {
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let parent = sessions.get(&subagent.parent_session_id).ok_or_else(|| {
                format!("unknown parent session_id {}", subagent.parent_session_id)
            })?;
            parent.handle.workspace_root.clone()
        };
        let mode = if subagent_tool_writes(tool_id) || tool_id == "shell.command" {
            ToolExecutionMode::ApplyWithPermission {
                permission_decision,
            }
        } else {
            ToolExecutionMode::ReadOnlyPreview
        };
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root,
            tool_call_id: tool_call_id.to_string(),
            tool_id: tool_id.to_string(),
            mode,
            args,
        })
        .map_err(|error| format!("{error:?}"))?;
        let mut child_sessions = self
            .subagents
            .sessions_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let child = child_sessions
            .get_mut(subagent_id)
            .ok_or_else(|| format!("unknown subagent session {subagent_id}"))?;
        child
            .record_runtime_event(
                "subagent.tool_completed",
                Actor::Tool,
                format!(
                    "{{\"subagent_id\":\"{}\",\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"ok\":{},\"preview\":\"{}\"}}",
                    json_escape(subagent_id),
                    json_escape(tool_call_id),
                    json_escape(tool_id),
                    result.ok,
                    json_escape(&trim_for_memory(&result.preview, 240))
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        Ok(result)
    }

    pub fn run_task_dispatch_with_transport<T: LiveHttpTransport>(
        &self,
        transport: &T,
        parent_session_id: &str,
        tool_call_id: &str,
        args: ToolExecutionArgs,
        endpoint: NativeProviderEndpoint,
    ) -> Result<SubagentSummary, String> {
        let prompt = args
            .content
            .as_deref()
            .or(args.query.as_deref())
            .ok_or_else(|| "task.dispatch requires prompt".to_string())?;
        let write_scope = task_dispatch_write_scope_paths(args.write_scope_json.as_deref())?;
        {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let parent = sessions
                .get_mut(parent_session_id)
                .ok_or_else(|| format!("unknown session_id {parent_session_id}"))?;
            parent
                .session
                .record_tool_call_requested(tool_call_id, "task.dispatch")
                .map_err(|error| format!("{error:?}"))?;
        }
        let agent_type = match args.model_role.as_deref() {
            Some("reviewer") => SubagentType::Reviewer,
            Some("executor") if !write_scope.is_empty() => SubagentType::Worker,
            Some("executor") => SubagentType::Explorer,
            Some("compactor") | None => SubagentType::Explorer,
            Some(other) => {
                let reason = format!("unsupported task.dispatch model_role {other}");
                self.record_task_dispatch_parent_result(
                    parent_session_id,
                    tool_call_id,
                    false,
                    &reason,
                )?;
                return Err(reason);
            }
        };
        let request = if matches!(agent_type, SubagentType::Worker) {
            SubagentRequest {
                agent_type,
                task: prompt.to_string(),
                model_family: NativeModelFamily::DeepSeek,
                tool_allowlist: SubagentType::Worker.default_tool_allowlist(),
                write_scope,
                worktree_required: true,
                worktree_ready: true,
                context_pack: crate::subagent::ContextPack::new(parent_session_id, prompt),
            }
        } else {
            SubagentRequest::readonly(
                parent_session_id,
                agent_type,
                prompt.to_string(),
                NativeModelFamily::DeepSeek,
            )
        };
        let subagent = self.spawn_subagent(parent_session_id, request)?;
        let summary = match self.run_subagent_model_task_with_transport(
            transport,
            &subagent.subagent_id,
            prompt,
            endpoint,
        ) {
            Ok(summary) => summary,
            Err(error) => {
                self.record_task_dispatch_parent_result(
                    parent_session_id,
                    tool_call_id,
                    false,
                    &error,
                )?;
                return Err(error);
            }
        };
        self.record_task_dispatch_parent_result(
            parent_session_id,
            tool_call_id,
            summary.status == SubagentStatus::Completed,
            &summary.summary,
        )?;
        Ok(summary)
    }

    pub fn execute_session_tool(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_id: &str,
        args: ToolExecutionArgs,
    ) -> Result<FacadeToolOutcome, String> {
        let mut args = args;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        // If the session was Completed and a new turn is starting,
        // reset plan_mode_active so the new turn is not blocked.
        if record.session.state() == AgentState::Completed {
            record.plan_mode_active = false;
        }
        if record.plan_mode_active && plan_mode_denies_tool(tool_id) {
            let reason = format!("PlanMode blocks {tool_id}; only read/search/repo/git/todo/context/plan.write are allowed");
            record
                .session
                .record_runtime_event(
                    "tool.blocked_by_plan_mode",
                    Actor::Runtime,
                    format!(
                        "{{\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"reason\":\"{}\"}}",
                        json_escape(tool_call_id),
                        json_escape(tool_id),
                        json_escape(&reason)
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            remember_session(record, format!("planmode blocked {tool_id}: {reason}"));
            return Ok(FacadeToolOutcome::BlockedByPolicy(reason));
        }
        record
            .session
            .record_tool_call_requested(tool_call_id, tool_id)
            .map_err(|error| format!("{error:?}"))?;
        if tool_id == "plan.enter" {
            let plan_approval_id = format!("{tool_call_id}_plan_approval");
            record.plan_mode_active = true;
            record
                .session
                .record_runtime_event(
                    "plan.mode_entered",
                    Actor::Runtime,
                    format!(
                        "{{\"plan_approval_id\":\"{}\",\"session_id\":\"{}\"}}",
                        json_escape(&plan_approval_id),
                        json_escape(session_id)
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            let plan_goal = args
                .content
                .as_ref()
                .and_then(|content| extract_json_string_field(content, "goal"));
            record
                .session
                .request_plan_approval(plan_approval_id.clone(), plan_goal)
                .map_err(|error| format!("{error:?}"))?;
            return Ok(FacadeToolOutcome::RequiresPlanApproval { plan_approval_id });
        }
        if tool_id == "plan.exit" {
            record.plan_mode_active = false;
            let result = ToolExecutionResult {
                tool_call_id: tool_call_id.to_string(),
                tool_id: tool_id.to_string(),
                ok: true,
                preview: "plan mode exited".to_string(),
                detail_json: "{\"plan_mode\":\"exit\"}".to_string(),
                exit_code: None,
            };
            record
                .session
                .record_tool_call_completed(&result.tool_call_id, &result.tool_id, result.ok)
                .and_then(|_| {
                    record.session.record_tool_result_artifact(
                        &result.tool_call_id,
                        &result.tool_id,
                        format!("artifact_{}", result.tool_call_id),
                        stable_text_hash(&result.detail_json),
                        &result.preview,
                    )
                })
                .map_err(|error| format!("{error:?}"))?;
            return Ok(FacadeToolOutcome::Executed(result));
        }
        if tool_id == "plan.write" {
            let content = args
                .content
                .clone()
                .or(args.edits_json.clone())
                .unwrap_or_else(|| "No plan content provided.".to_string());
            let path = record
                .handle
                .workspace_root
                .join(".researchcode")
                .join("plans")
                .join(format!("{}.md", record.handle.session_id));
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            }
            fs::write(&path, &content).map_err(|error| error.to_string())?;
            let rel = path
                .strip_prefix(&record.handle.workspace_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let result = ToolExecutionResult {
                tool_call_id: tool_call_id.to_string(),
                tool_id: tool_id.to_string(),
                ok: true,
                preview: format!("plan.write wrote {rel}"),
                detail_json: format!(
                    "{{\"path\":\"{}\",\"content_hash\":\"{}\"}}",
                    json_escape(&rel),
                    stable_text_hash(&content)
                ),
                exit_code: None,
            };
            record
                .session
                .record_tool_call_completed(&result.tool_call_id, &result.tool_id, result.ok)
                .and_then(|_| {
                    record.session.record_tool_result_artifact(
                        &result.tool_call_id,
                        &result.tool_id,
                        format!("artifact_{}", result.tool_call_id),
                        stable_text_hash(&result.detail_json),
                        &result.preview,
                    )
                })
                .map_err(|error| format!("{error:?}"))?;
            remember_session(record, result.preview.clone());
            return Ok(FacadeToolOutcome::Executed(result));
        }
        if tool_id == "ask_user" {
            let question = args
                .query
                .clone()
                .or(args.content.clone())
                .unwrap_or_else(|| "The agent needs clarification before continuing.".to_string());
            let result = ToolExecutionResult {
                tool_call_id: tool_call_id.to_string(),
                tool_id: tool_id.to_string(),
                ok: true,
                preview: format!(
                    "ask_user waiting for user: {}",
                    trim_for_memory(&question, 160)
                ),
                detail_json: format!(
                    "{{\"question\":\"{}\",\"status\":\"waiting_for_user\"}}",
                    json_escape(&question)
                ),
                exit_code: None,
            };
            record
                .session
                .record_tool_call_completed(&result.tool_call_id, &result.tool_id, result.ok)
                .and_then(|_| {
                    record.session.record_tool_result_artifact(
                        &result.tool_call_id,
                        &result.tool_id,
                        format!("artifact_{}", result.tool_call_id),
                        stable_text_hash(&result.detail_json),
                        &result.preview,
                    )
                })
                .and_then(|_| {
                    record.session.record_runtime_event(
                        "user.question_requested",
                        Actor::Runtime,
                        format!(
                            "{{\"tool_call_id\":\"{}\",\"question\":\"{}\"}}",
                            json_escape(tool_call_id),
                            json_escape(&question)
                        ),
                    )
                })
                .and_then(|_| record.session.transition_to(AgentState::WaitingForUser))
                .map_err(|error| format!("{error:?}"))?;
            remember_session(
                record,
                format!(
                    "waiting for user clarification: {}",
                    trim_for_memory(&question, 240)
                ),
            );
            return Ok(FacadeToolOutcome::Executed(result));
        }
        inject_base_hash_from_last_read(record, tool_id, &mut args);
        if let Some(reason) = read_before_write_violation(record, tool_id, &args) {
            record
                .session
                .record_tool_call_completed(tool_call_id, tool_id, false)
                .and_then(|_| {
                    record.session.record_tool_result_artifact(
                        tool_call_id,
                        tool_id,
                        format!("artifact_{tool_call_id}"),
                        stable_text_hash(&reason),
                        reason.clone(),
                    )
                })
                .map_err(|error| format!("{error:?}"))?;
            remember_session(record, format!("blocked {tool_id}: {reason}"));
            return Ok(FacadeToolOutcome::BlockedByPolicy(reason));
        }
        let mode = self.permissions.apply_permission_policy(
            record,
            facade_tool_mode(record.handle.autonomy_mode, tool_id, &args),
            tool_id,
            &args,
        )?;
        match mode {
            FacadeToolMode::Preview => {
                let workspace_root = record.handle.workspace_root.clone();
                let executable_args = args;
                drop(sessions);
                let result = match execute_tool(&ToolExecutionRequest {
                    workspace_root,
                    tool_call_id: tool_call_id.to_string(),
                    tool_id: tool_id.to_string(),
                    mode: ToolExecutionMode::ReadOnlyPreview,
                    args: executable_args,
                }) {
                    Ok(result) => result,
                    Err(error) => tool_error_to_result(tool_call_id, tool_id, &error),
                };
                let mut sessions = self
                    .sessions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let record = sessions
                    .get_mut(session_id)
                    .ok_or_else(|| format!("unknown session_id {session_id}"))?;
                record
                    .session
                    .record_tool_call_completed(&result.tool_call_id, &result.tool_id, result.ok)
                    .and_then(|_| {
                        record.session.record_tool_result_artifact(
                            &result.tool_call_id,
                            &result.tool_id,
                            format!("artifact_{}", result.tool_call_id),
                            stable_text_hash(&result.detail_json),
                            &result.preview,
                        )
                    })
                    .map_err(|error| format!("{error:?}"))?;
                update_runtime_tool_state(record, &result);
                Ok(FacadeToolOutcome::Executed(result))
            }
            FacadeToolMode::FastAutoApply => {
                let patch_record = if tool_id == "patch.apply" {
                    Some(record_patch_proposal_validation(
                        &mut record.session,
                        &record.handle.workspace_root,
                        tool_call_id,
                        &args,
                    )?)
                } else {
                    None
                };
                let request_type = permission_request_type_for_tool(tool_id)
                    .ok_or_else(|| format!("missing permission type for {tool_id}"))?;
                let permission_id = format!("{tool_call_id}_fast_auto_permission");
                record
                    .session
                    .request_permission(permission_id, request_type, Some(tool_id.to_string()))
                    .and_then(|_| {
                        record
                            .session
                            .decide_permission(PermissionDecisionKind::AllowProjectRule)
                    })
                    .map_err(|error| format!("{error:?}"))?;
                let workspace_root = record.handle.workspace_root.clone();
                let executable_args = args;
                drop(sessions);
                let result = execute_tool(&ToolExecutionRequest {
                    workspace_root,
                    tool_call_id: tool_call_id.to_string(),
                    tool_id: tool_id.to_string(),
                    mode: ToolExecutionMode::ApplyWithPermission {
                        permission_decision: Some(PermissionDecisionKind::AllowOnce),
                    },
                    args: executable_args,
                })
                .map_err(|error| format!("{error:?}"))?;
                let mut sessions = self
                    .sessions
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                let record = sessions
                    .get_mut(session_id)
                    .ok_or_else(|| format!("unknown session_id {session_id}"))?;
                record
                    .session
                    .record_tool_call_completed(&result.tool_call_id, &result.tool_id, result.ok)
                    .and_then(|_| {
                        record.session.record_tool_result_artifact(
                            &result.tool_call_id,
                            &result.tool_id,
                            format!("artifact_{}", result.tool_call_id),
                            stable_text_hash(&result.detail_json),
                            &result.preview,
                        )
                    })
                    .map_err(|error| format!("{error:?}"))?;
                update_runtime_tool_state(record, &result);
                if let Some((patch_id, path)) = patch_record {
                    if result.ok {
                        record
                            .session
                            .record_patch_applied(patch_id, path)
                            .map_err(|error| format!("{error:?}"))?;
                    }
                }
                if matches!(
                    record.session.state(),
                    AgentState::ApplyingPatch | AgentState::RunningCommand
                ) {
                    record
                        .session
                        .transition_to(AgentState::Executing)
                        .map_err(|error| format!("{error:?}"))?;
                }
                Ok(FacadeToolOutcome::Executed(result))
            }
            FacadeToolMode::RequirePermission(request_type) => {
                let permission_id = format!("{tool_call_id}_permission");
                record
                    .session
                    .request_permission(
                        permission_id.clone(),
                        request_type.clone(),
                        Some(tool_id.to_string()),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                record_permission_context_event(
                    &mut record.session,
                    &permission_id,
                    tool_id,
                    &request_type,
                    &args,
                )?;
                Ok(FacadeToolOutcome::RequiresPermission {
                    permission_id,
                    request_type,
                })
            }
            FacadeToolMode::Blocked(reason) => {
                record
                    .session
                    .record_tool_call_completed(tool_call_id, tool_id, false)
                    .and_then(|_| {
                        record.session.record_tool_result_artifact(
                            tool_call_id,
                            tool_id,
                            format!("artifact_{tool_call_id}"),
                            stable_text_hash(&reason),
                            reason.clone(),
                        )
                    })
                    .map_err(|error| format!("{error:?}"))?;
                remember_session(
                    record,
                    format!("blocked {tool_id}: {}", trim_for_memory(&reason, 240)),
                );
                Ok(FacadeToolOutcome::BlockedByPolicy(reason))
            }
        }
    }

    pub fn continue_session_tool_after_permission(
        &self,
        session_id: &str,
        tool_call_id: &str,
        tool_id: &str,
        args: ToolExecutionArgs,
        decision: PermissionDecisionKind,
    ) -> Result<FacadeToolOutcome, String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        let denied = matches!(decision, PermissionDecisionKind::Deny);
        let request_type = permission_request_type_for_tool(tool_id);
        let normalized_summary = normalized_permission_summary(tool_id, &args);
        record
            .session
            .decide_permission(decision.clone())
            .map_err(|error| format!("{error:?}"))?;
        self.permissions.persist_permission_decision_rule(
            record,
            request_type,
            tool_id,
            &normalized_summary,
            &decision,
        )?;
        if denied {
            record
                .session
                .record_tool_call_completed(tool_call_id, tool_id, false)
                .and_then(|_| {
                    record.session.record_tool_result_artifact(
                        tool_call_id,
                        tool_id,
                        format!("artifact_{tool_call_id}"),
                        stable_text_hash("permission denied"),
                        "permission denied",
                    )
                })
                .map_err(|error| format!("{error:?}"))?;
            return Ok(FacadeToolOutcome::BlockedByPolicy(
                "permission denied".to_string(),
            ));
        }
        let patch_record = if tool_id == "patch.apply" {
            Some(record_patch_proposal_validation(
                &mut record.session,
                &record.handle.workspace_root,
                tool_call_id,
                &args,
            )?)
        } else {
            None
        };
        let workspace_root = record.handle.workspace_root.clone();
        let executable_args = args;
        drop(sessions);
        let result = execute_tool(&ToolExecutionRequest {
            workspace_root,
            tool_call_id: tool_call_id.to_string(),
            tool_id: tool_id.to_string(),
            mode: ToolExecutionMode::ApplyWithPermission {
                permission_decision: Some(decision),
            },
            args: executable_args,
        })
        .map_err(|error| format!("{error:?}"))?;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        record
            .session
            .record_tool_call_completed(&result.tool_call_id, &result.tool_id, result.ok)
            .and_then(|_| {
                record.session.record_tool_result_artifact(
                    &result.tool_call_id,
                    &result.tool_id,
                    format!("artifact_{}", result.tool_call_id),
                    stable_text_hash(&result.detail_json),
                    &result.preview,
                )
            })
            .map_err(|error| format!("{error:?}"))?;
        update_runtime_tool_state(record, &result);
        if let Some((patch_id, path)) = patch_record {
            if result.ok {
                record
                    .session
                    .record_patch_applied(patch_id, path)
                    .map_err(|error| format!("{error:?}"))?;
            }
        }
        if matches!(
            record.session.state(),
            AgentState::ApplyingPatch | AgentState::RunningCommand
        ) {
            record
                .session
                .transition_to(AgentState::Executing)
                .map_err(|error| format!("{error:?}"))?;
        }
        Ok(FacadeToolOutcome::Executed(result))
    }

    pub fn build_context_bundle(&self, session_id: &str) -> Result<ContextBundle, String> {
        self.sessions
            .with_ref(session_id, |record| {
                self.context.build_context_bundle(record)
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))?
    }

    pub fn conversation_history_openai_json(&self, session_id: &str) -> Result<String, String> {
        self.sessions
            .with_ref(session_id, |record| {
                self.context.conversation_history_openai_json(record)
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))
    }

    fn build_native_prompt_with_runtime_context(
        &self,
        session_id: &str,
        prompt: &str,
        evidence_directive: &str,
        context_item_char_limit: usize,
    ) -> Result<String, String> {
        let context = self.build_context_bundle(session_id)?;
        let context_preview = context
            .items
            .iter()
            .map(|item| {
                format!(
                    "[{:?}] {}:\n{}",
                    item.kind,
                    item.source,
                    trim_for_memory(&item.content, context_item_char_limit)
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let conversation_history = self.conversation_history_openai_json(session_id)?;
        let conversation_section = if conversation_history.trim() == "[]" {
            String::new()
        } else {
            format!(
                "\n\n# Conversation History (OpenAI JSON)\nThe following JSON array is the authoritative cross-turn conversation history. It preserves assistant tool_calls, tool_call_id-bound tool results, and reasoning_content when available. Use it instead of legacy text conversation summaries.\n{}",
                conversation_history
            )
        };
        Ok(format!(
            "{prompt}\n\n# Runtime Context\n{context_preview}{conversation_section}{evidence_directive}"
        ))
    }

    pub fn export_events(&self, session_id: &str, path: &Path) -> Result<(), String> {
        let events = self.stream_agent_events(session_id)?.jsonl;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        EventLog::import_jsonl(&events).map_err(|error| format!("{error:?}"))?;
        fs::write(path, events).map_err(|error| error.to_string())
    }

    pub fn get_session_snapshot(&self, session_id: &str) -> Result<RuntimeSessionSnapshot, String> {
        self.sessions
            .with_ref(session_id, |record| {
                let event_log = EventLog::import_jsonl(&record.session.export_events_jsonl())
                    .map_err(|error| format!("{error:?}"))?;
                let approval_queue = extract_approval_queue(&event_log);
                Ok(RuntimeSessionSnapshot {
                    session_id: session_id.to_string(),
                    state: record.session.state(),
                    event_count: record.session.event_count(),
                    model_mode: record.handle.model_mode,
                    autonomy_mode: record.handle.autonomy_mode,
                    pending_permission_count: approval_queue.permissions.len(),
                    pending_plan_approval_count: approval_queue.plan_approvals.len(),
                    approval_queue,
                    workspace_root: record.handle.workspace_root.clone(),
                    artifact_root: record.handle.artifact_root.clone(),
                    plan_mode_active: record.plan_mode_active,
                    session_memory_count: record.session_memory.len(),
                })
            })
            .ok_or_else(|| format!("unknown session_id {session_id}"))?
    }

    pub fn close_session(&self, session_id: &str) -> Result<(), String> {
        self.sessions.remove(session_id);
        Ok(())
    }

    pub fn resume_session_from_eventlog(
        &self,
        path: &Path,
    ) -> Result<RuntimeSessionHandle, String> {
        let event_log = EventLog::read_jsonl(path).map_err(|error| format!("{error:?}"))?;
        let session =
            AgentSession::resume_from_event_log(event_log).map_err(|error| format!("{error:?}"))?;
        let session_id = extract_session_id_from_jsonl(&session.export_events_jsonl())
            .ok_or_else(|| "resumed event log missing session_id".to_string())?;
        let handle = RuntimeSessionHandle {
            project_id: "local".to_string(),
            session_id: session_id.clone(),
            task_id: "resumed_task".to_string(),
            workspace_root: self.workspace_root.clone(),
            artifact_root: self.artifact_root.join(&session_id),
            model_mode: RuntimeModelMode::DeepSeek,
            autonomy_mode: AutonomyMode::FastAuto,
        };
        self.sessions.insert(
            session_id,
            RuntimeSessionRecord {
                handle: handle.clone(),
                session,
                session_policy: PermissionRuleSet::default(),
                session_memory: vec!["resumed from event log".to_string()],
                file_state: HashMap::new(),
                plan_mode_active: false,
                repeated_tool_failures: HashMap::new(),
                path_corrections: HashMap::new(),
                discovered_roots: Vec::new(),
                native_tool_completion: HashMap::new(),
                error_recovery: ErrorRecoveryState::default(),
                pending_native_decision: None,
            },
        );
        Ok(handle)
    }

    pub fn spawn_subagent(
        &self,
        parent_session_id: &str,
        request: SubagentRequest,
    ) -> Result<SubagentSession, String> {
        validate_subagent_request(&request)?;
        let nonce = monotonic_nonce()?;
        let subagent_id = format!("subagent_{}_{}", request.agent_type.as_str(), nonce);
        let mut child_session =
            AgentSession::new("local", subagent_id.clone(), request.task.clone())
                .map_err(|error| format!("{error:?}"))?;
        let session = SubagentSession {
            subagent_id: subagent_id.clone(),
            parent_session_id: parent_session_id.to_string(),
            agent_type: request.agent_type.clone(),
            model_family: request.model_family,
            tool_allowlist: if request.tool_allowlist.is_empty() {
                request.agent_type.default_tool_allowlist()
            } else {
                request.tool_allowlist.clone()
            },
            write_scope: request.write_scope.clone(),
            context_pack_id: request.context_pack.context_pack_id.clone(),
            status: SubagentStatus::Created,
            event_log_ref: format!("subagents/{subagent_id}.jsonl"),
            summary: None,
        };
        child_session
            .record_runtime_event(
                "subagent.child_created",
                Actor::Runtime,
                format!(
                    "{{\"subagent_id\":\"{}\",\"parent_session_id\":\"{}\",\"agent_type\":\"{}\",\"context_pack_id\":\"{}\",\"event_log_ref\":\"{}\"}}",
                    json_escape(&session.subagent_id),
                    json_escape(parent_session_id),
                    json_escape(session.agent_type.as_str()),
                    json_escape(&session.context_pack_id),
                    json_escape(&session.event_log_ref)
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let parent = sessions
                .get_mut(parent_session_id)
                .ok_or_else(|| format!("unknown parent session_id {parent_session_id}"))?;
            parent
                .session
                .record_runtime_event(
                    "subagent.spawned",
                    Actor::Runtime,
                    format!(
                        "{{\"subagent_id\":\"{}\",\"agent_type\":\"{}\",\"context_pack_id\":\"{}\",\"status\":\"{}\"}}",
                        json_escape(&session.subagent_id),
                        json_escape(session.agent_type.as_str()),
                        json_escape(&session.context_pack_id),
                        json_escape(session.status.as_str())
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            remember_session(
                parent,
                format!(
                    "spawned subagent {} type={} task={}",
                    session.subagent_id,
                    session.agent_type.as_str(),
                    trim_for_memory(&request.task, 180)
                ),
            );
        }
        self.subagents.insert_subagent(subagent_id, session.clone());
        self.subagents
            .insert_session(session.subagent_id.clone(), child_session);
        Ok(session)
    }

    pub fn send_subagent_message(&self, subagent_id: &str, message: &str) -> Result<(), String> {
        let mut subagents = self
            .subagents
            .subagents_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let subagent = subagents
            .get_mut(subagent_id)
            .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?;
        if matches!(
            subagent.status,
            SubagentStatus::Completed | SubagentStatus::Failed | SubagentStatus::Cancelled
        ) {
            return Err(format!(
                "subagent {subagent_id} is terminal: {}",
                subagent.status.as_str()
            ));
        }
        subagent.status = SubagentStatus::Running;
        let parent_session_id = subagent.parent_session_id.clone();
        drop(subagents);
        {
            let mut child_sessions = self
                .subagents
                .sessions_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let child = child_sessions
                .get_mut(subagent_id)
                .ok_or_else(|| format!("unknown subagent session {subagent_id}"))?;
            child
                .record_runtime_event(
                    "subagent.message_received",
                    Actor::Runtime,
                    format!(
                        "{{\"subagent_id\":\"{}\",\"message_preview\":\"{}\"}}",
                        json_escape(subagent_id),
                        json_escape(&trim_for_memory(message, 240))
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
        }
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let parent = sessions
            .get_mut(&parent_session_id)
            .ok_or_else(|| format!("unknown parent session_id {parent_session_id}"))?;
        parent
            .session
            .record_runtime_event(
                "subagent.message_sent",
                Actor::Runtime,
                format!(
                    "{{\"subagent_id\":\"{}\",\"message_preview\":\"{}\"}}",
                    json_escape(subagent_id),
                    json_escape(&trim_for_memory(message, 240))
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        remember_session(
            parent,
            format!(
                "subagent {subagent_id} message: {}",
                trim_for_memory(message, 180)
            ),
        );
        Ok(())
    }

    pub fn run_subagent_task(
        &self,
        subagent_id: &str,
        message: &str,
    ) -> Result<SubagentSummary, String> {
        self.send_subagent_message(subagent_id, message)?;
        let subagent = {
            let subagents = self
                .subagents
                .subagents_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            subagents
                .get(subagent_id)
                .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?
                .clone()
        };
        let workspace_root = {
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let parent = sessions.get(&subagent.parent_session_id).ok_or_else(|| {
                format!("unknown parent session_id {}", subagent.parent_session_id)
            })?;
            parent.handle.workspace_root.clone()
        };
        let mut evidence_refs = Vec::new();
        let mut previews = Vec::new();
        let mut run_tool = |tool_id: &str, args: ToolExecutionArgs| -> Result<(), String> {
            if self.subagent_is_cancelled(subagent_id)? {
                return Err(format!("subagent {subagent_id} cancelled"));
            }
            if !subagent.tool_allowlist.iter().any(|item| item == tool_id) {
                return Ok(());
            }
            let tool_call_id = format!(
                "{}_{}_{}",
                subagent_id,
                tool_id.replace('.', "_"),
                evidence_refs.len() + 1
            );
            let result = execute_tool(&ToolExecutionRequest {
                workspace_root: workspace_root.clone(),
                tool_call_id: tool_call_id.clone(),
                tool_id: tool_id.to_string(),
                mode: ToolExecutionMode::ReadOnlyPreview,
                args,
            })
            .unwrap_or_else(|error| tool_error_to_result(&tool_call_id, tool_id, &error));
            evidence_refs.push(format!(
                "subagent:{}:{}:{}",
                subagent_id,
                tool_id,
                stable_text_hash(&result.detail_json)
            ));
            previews.push(format!(
                "{} ok={} {}",
                tool_id,
                result.ok,
                trim_for_memory(&result.preview, 180)
            ));
            let mut child_sessions = self
                .subagents
                .sessions_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(child) = child_sessions.get_mut(subagent_id) {
                child
                    .record_runtime_event(
                        "subagent.tool_completed",
                        Actor::Tool,
                        format!(
                            "{{\"subagent_id\":\"{}\",\"tool_id\":\"{}\",\"ok\":{},\"preview\":\"{}\"}}",
                            json_escape(subagent_id),
                            json_escape(tool_id),
                            result.ok,
                            json_escape(&trim_for_memory(&result.preview, 240))
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
            Ok(())
        };

        run_tool(
            "repo.map",
            ToolExecutionArgs {
                root: Some(".".to_string()),
                ..ToolExecutionArgs::default()
            },
        )?;
        if let Some(path) = infer_path_from_subagent_message(message) {
            let normalized = normalize_user_supplied_path(&path);
            let absolute = if Path::new(&normalized).is_absolute() {
                PathBuf::from(&normalized)
            } else {
                workspace_root.join(&normalized)
            };
            if absolute.is_dir() {
                run_tool(
                    "repo.map",
                    ToolExecutionArgs {
                        root: Some(absolute.to_string_lossy().to_string()),
                        ..ToolExecutionArgs::default()
                    },
                )?;
            } else {
                run_tool(
                    "file.read",
                    ToolExecutionArgs {
                        path: Some(normalized),
                        max_bytes: Some(12_000),
                        ..ToolExecutionArgs::default()
                    },
                )?;
            }
        }
        if let Some(pattern) = infer_search_pattern(message) {
            run_tool(
                "search.ripgrep",
                ToolExecutionArgs {
                    root: Some(".".to_string()),
                    pattern: Some(pattern),
                    ..ToolExecutionArgs::default()
                },
            )?;
        }
        run_tool(
            "git.status",
            ToolExecutionArgs {
                root: Some(".".to_string()),
                ..ToolExecutionArgs::default()
            },
        )?;

        let summary = SubagentSummary {
            subagent_id: subagent.subagent_id.clone(),
            agent_type: subagent.agent_type.clone(),
            status: SubagentStatus::Completed,
            summary: format!(
                "{} executed {} read-only tools: {}",
                subagent.agent_type.as_str(),
                previews.len(),
                trim_for_memory(&previews.join("; "), 420)
            ),
            evidence_refs,
        };
        {
            let mut subagents = self
                .subagents
                .subagents_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(subagent) = subagents.get_mut(subagent_id) {
                subagent.status = SubagentStatus::Completed;
                subagent.summary = Some(summary.clone());
            }
        }
        self.record_subagent_summary(&summary)?;
        Ok(summary)
    }

    pub fn run_subagent_model_task_with_transport<T: LiveHttpTransport>(
        &self,
        transport: &T,
        subagent_id: &str,
        message: &str,
        endpoint: NativeProviderEndpoint,
    ) -> Result<SubagentSummary, String> {
        self.send_subagent_message(subagent_id, message)?;
        let subagent = {
            let subagents = self
                .subagents
                .subagents_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            subagents
                .get(subagent_id)
                .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?
                .clone()
        };
        let read_only_child = matches!(
            subagent.agent_type,
            SubagentType::Explorer | SubagentType::Reviewer | SubagentType::Reproducer
        ) && subagent.write_scope.is_empty();
        let scoped_worker =
            matches!(subagent.agent_type, SubagentType::Worker) && !subagent.write_scope.is_empty();
        if !read_only_child && !scoped_worker {
            return Err(
                "LLM-driven subagent loop requires a read-only child or a worker with write_scope"
                    .to_string(),
            );
        }
        if subagent.model_family != NativeModelFamily::DeepSeek {
            return Err("LLM-driven subagent loop currently requires DeepSeek".to_string());
        }
        let (workspace_root, artifact_root) = {
            let sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let parent = sessions.get(&subagent.parent_session_id).ok_or_else(|| {
                format!("unknown parent session_id {}", subagent.parent_session_id)
            })?;
            (
                parent.handle.workspace_root.clone(),
                parent.handle.artifact_root.clone(),
            )
        };
        let turn_id = format!("{}_model_turn_{}", subagent_id, monotonic_nonce()?);
        {
            let mut child_sessions = self
                .subagents
                .sessions_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let child = child_sessions
                .get_mut(subagent_id)
                .ok_or_else(|| format!("unknown subagent session {subagent_id}"))?;
            child
                .record_runtime_event(
                    "subagent.model_turn_started",
                    Actor::Runtime,
                    format!(
                        "{{\"subagent_id\":\"{}\",\"turn_id\":\"{}\",\"model_family\":\"deepseek\",\"tool_exposure\":\"{}\"}}",
                        json_escape(subagent_id),
                        json_escape(&turn_id),
                        if scoped_worker { "code_edit" } else { "read_only" }
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
        }
        let mut endpoint = endpoint;
        endpoint.live_calls_enabled_by_default = true;
        let mut hook_dispatcher = None;
        if scoped_worker {
            let mut dispatcher = HookDispatcher::new(HookDispatchPolicy::default());
            dispatcher.register(Arc::new(SubagentWriteScopeHook {
                write_scope: subagent.write_scope.clone(),
            }));
            hook_dispatcher = Some(dispatcher);
        }
        let prompt = if scoped_worker {
            format!(
                "Subagent type: {}\nParent session: {}\nWrite scope: {}\nTask: {}\n\nUse code-edit tools only inside the listed write scope. Return a concise evidence-grounded summary.",
                subagent.agent_type.as_str(),
                subagent.parent_session_id,
                subagent.write_scope.join(", "),
                message
            )
        } else {
            format!(
                "Subagent type: {}\nParent session: {}\nTask: {}\n\nReturn a concise evidence-grounded summary. Use read-only tools only.",
                subagent.agent_type.as_str(),
                subagent.parent_session_id,
                message
            )
        };
        let request = NativeAgentLoopV2Request {
            project_id: "local".to_string(),
            session_id: subagent_id.to_string(),
            task_id: format!("{subagent_id}_task"),
            turn_id: Some(turn_id.clone()),
            workspace_root,
            artifact_root,
            endpoint,
            prompt,
            max_tokens: 512,
            max_iterations: if scoped_worker { 3 } else { 2 },
            max_tool_calls: if scoped_worker { 6 } else { 4 },
            tool_exposure: if scoped_worker {
                NativeAgentToolExposure::CodeEdit
            } else {
                NativeAgentToolExposure::ReadOnly
            },
            permission_mode: if scoped_worker {
                PermissionMode::BypassPermissions
            } else {
                PermissionMode::Default
            },
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: Some(ErrorRecoveryState::default()),
            hook_dispatcher,
            concurrent_tool_execution: false,
        };
        let result = AgentKernel::for_request(&request).run_turn(transport, request, None)?;
        let imported =
            EventLog::import_jsonl(&result.event_jsonl).map_err(|error| format!("{error:?}"))?;
        {
            let mut child_sessions = self
                .subagents
                .sessions_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let child = child_sessions
                .get_mut(subagent_id)
                .ok_or_else(|| format!("unknown subagent session {subagent_id}"))?;
            child.merge_events(&imported.iter().cloned().collect::<Vec<_>>());
            child
                .record_runtime_event(
                    "subagent.model_turn_completed",
                    Actor::Runtime,
                    format!(
                        "{{\"subagent_id\":\"{}\",\"turn_id\":\"{}\",\"status\":\"{:?}\",\"models\":{},\"tools\":{}}}",
                        json_escape(subagent_id),
                        json_escape(&turn_id),
                        result.status,
                        result.model_call_count,
                        result.tool_call_count
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            child
                .set_state(result.final_state)
                .map_err(|error| format!("{error:?}"))?;
        }
        let summary = SubagentSummary {
            subagent_id: subagent.subagent_id.clone(),
            agent_type: subagent.agent_type.clone(),
            status: if result.status == NativeAgentLoopStatus::Completed {
                SubagentStatus::Completed
            } else {
                SubagentStatus::Failed
            },
            summary: format!(
                "llm child status={:?} models={} tools={}",
                result.status, result.model_call_count, result.tool_call_count
            ),
            evidence_refs: vec![format!(
                "subagent:{}:model_loop:{}",
                subagent_id,
                stable_text_hash(&result.event_jsonl)
            )],
        };
        {
            let mut subagents = self
                .subagents
                .subagents_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(subagent) = subagents.get_mut(subagent_id) {
                subagent.status = summary.status.clone();
                subagent.summary = Some(summary.clone());
            }
        }
        self.record_subagent_summary(&summary)?;
        Ok(summary)
    }

    pub fn stream_subagent_events(
        &self,
        subagent_id: &str,
    ) -> Result<RuntimeAgentEventStream, String> {
        let child_sessions = self
            .subagents
            .sessions_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let child = child_sessions
            .get(subagent_id)
            .ok_or_else(|| format!("unknown subagent session {subagent_id}"))?;
        Ok(RuntimeAgentEventStream {
            session_id: subagent_id.to_string(),
            jsonl: child.export_events_jsonl(),
        })
    }

    pub fn resume_subagent(&self, subagent_id: &str) -> Result<SubagentSession, String> {
        let mut subagents = self
            .subagents
            .subagents_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let subagent = subagents
            .get_mut(subagent_id)
            .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?;
        if subagent.status != SubagentStatus::Completed {
            subagent.status = SubagentStatus::Running;
        }
        Ok(subagent.clone())
    }

    pub fn cancel_subagent(&self, subagent_id: &str) -> Result<SubagentSession, String> {
        let mut subagents = self
            .subagents
            .subagents_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let subagent = subagents
            .get_mut(subagent_id)
            .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?;
        subagent.status = SubagentStatus::Cancelled;
        let cancelled = subagent.clone();
        let parent_session_id = cancelled.parent_session_id.clone();
        drop(subagents);
        {
            let mut child_sessions = self
                .subagents
                .sessions_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(child) = child_sessions.get_mut(subagent_id) {
                child
                    .record_runtime_event(
                        "subagent.cancelled",
                        Actor::Runtime,
                        format!("{{\"subagent_id\":\"{}\"}}", json_escape(subagent_id)),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
        }
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(parent) = sessions.get_mut(&parent_session_id) {
            parent
                .session
                .record_runtime_event(
                    "subagent.cancelled",
                    Actor::Runtime,
                    format!("{{\"subagent_id\":\"{}\"}}", json_escape(subagent_id)),
                )
                .map_err(|error| format!("{error:?}"))?;
        }
        Ok(cancelled)
    }

    pub fn summarize_subagent(&self, subagent_id: &str) -> Result<SubagentSummary, String> {
        let mut subagents = self
            .subagents
            .subagents_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let subagent = subagents
            .get_mut(subagent_id)
            .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?;
        subagent.status = SubagentStatus::Completed;
        if let Some(summary) = &subagent.summary {
            return Ok(summary.clone());
        }
        let summary = SubagentSummary {
            subagent_id: subagent.subagent_id.clone(),
            agent_type: subagent.agent_type.clone(),
            status: subagent.status.clone(),
            summary: format!(
                "{} completed with allowlist={}",
                subagent.agent_type.as_str(),
                subagent.tool_allowlist.join(",")
            ),
            evidence_refs: vec![format!("evidence:{}", subagent.context_pack_id)],
        };
        subagent.summary = Some(summary.clone());
        drop(subagents);
        self.record_subagent_summary(&summary)?;
        Ok(summary)
    }

    fn subagent_is_cancelled(&self, subagent_id: &str) -> Result<bool, String> {
        let subagents = self
            .subagents
            .subagents_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let subagent = subagents
            .get(subagent_id)
            .ok_or_else(|| format!("unknown subagent_id {subagent_id}"))?;
        Ok(subagent.status == SubagentStatus::Cancelled)
    }

    fn record_subagent_tool_blocked(
        &self,
        subagent_id: &str,
        tool_call_id: &str,
        tool_id: &str,
        reason: &str,
    ) -> Result<(), String> {
        let mut child_sessions = self
            .subagents
            .sessions_lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let child = child_sessions
            .get_mut(subagent_id)
            .ok_or_else(|| format!("unknown subagent session {subagent_id}"))?;
        child
            .record_runtime_event(
                "subagent.tool_blocked",
                Actor::Runtime,
                format!(
                    "{{\"subagent_id\":\"{}\",\"tool_call_id\":\"{}\",\"tool_id\":\"{}\",\"reason\":\"{}\"}}",
                    json_escape(subagent_id),
                    json_escape(tool_call_id),
                    json_escape(tool_id),
                    json_escape(reason)
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    fn record_task_dispatch_parent_result(
        &self,
        parent_session_id: &str,
        tool_call_id: &str,
        ok: bool,
        preview: &str,
    ) -> Result<(), String> {
        let detail_json = format!(
            "{{\"status\":\"{}\",\"preview\":{}}}",
            if ok { "completed" } else { "failed" },
            json_string(&trim_for_memory(preview, 480))
        );
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let parent = sessions
            .get_mut(parent_session_id)
            .ok_or_else(|| format!("unknown session_id {parent_session_id}"))?;
        parent
            .session
            .record_tool_call_completed(tool_call_id, "task.dispatch", ok)
            .and_then(|_| {
                parent.session.record_tool_result_artifact(
                    tool_call_id,
                    "task.dispatch",
                    format!("artifact_{tool_call_id}"),
                    stable_text_hash(&detail_json),
                    trim_for_memory(preview, 480),
                )
            })
            .map_err(|error| format!("{error:?}"))?;
        remember_session(
            parent,
            format!(
                "task.dispatch status={} {}",
                ok,
                trim_for_memory(preview, 180)
            ),
        );
        Ok(())
    }

    fn record_subagent_summary(&self, summary: &SubagentSummary) -> Result<(), String> {
        {
            let mut child_sessions = self
                .subagents
                .sessions_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(child) = child_sessions.get_mut(&summary.subagent_id) {
                child
                    .record_runtime_event(
                        "subagent.completed",
                        Actor::Runtime,
                        format!(
                            "{{\"subagent_id\":\"{}\",\"status\":\"{}\",\"summary\":\"{}\",\"evidence_refs\":[{}]}}",
                            json_escape(&summary.subagent_id),
                            json_escape(summary.status.as_str()),
                            json_escape(&summary.summary),
                            summary
                                .evidence_refs
                                .iter()
                                .map(|item| format!("\"{}\"", json_escape(item)))
                                .collect::<Vec<_>>()
                                .join(",")
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
        }
        let parent_session_id = {
            let subagents = self
                .subagents
                .subagents_lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            subagents
                .get(&summary.subagent_id)
                .ok_or_else(|| format!("unknown subagent_id {}", summary.subagent_id))?
                .parent_session_id
                .clone()
        };
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(parent) = sessions.get_mut(&parent_session_id) {
            parent
                .session
                .record_runtime_event(
                    "subagent.summary_recorded",
                    Actor::Runtime,
                    format!(
                        "{{\"subagent_id\":\"{}\",\"status\":\"{}\",\"summary\":\"{}\",\"evidence_refs\":[{}]}}",
                        json_escape(&summary.subagent_id),
                        json_escape(summary.status.as_str()),
                        json_escape(&summary.summary),
                        summary
                            .evidence_refs
                            .iter()
                            .map(|item| format!("\"{}\"", json_escape(item)))
                            .collect::<Vec<_>>()
                            .join(",")
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            remember_session(
                parent,
                format!(
                    "subagent summary {}: {}",
                    summary.subagent_id, summary.summary
                ),
            );
        }
        Ok(())
    }

    pub fn run_ultraplan_fixture(
        &self,
        session_id: &str,
        goal: &str,
    ) -> Result<UltraPlanSpec, String> {
        let (team, ledger, plan) = build_ultraplan_fixture(goal);
        plan.validate(&ledger)?;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        if record.handle.model_mode != RuntimeModelMode::DeepSeek {
            return Err("UltraPlan v1 requires DeepSeek native mode".to_string());
        }
        record
            .session
            .record_runtime_event(
                "agentteam.completed",
                Actor::Runtime,
                format!(
                    "{{\"team_id\":\"{}\",\"mode\":\"{}\",\"status\":\"{}\",\"evidence_count\":{}}}",
                    json_escape(&team.team_id),
                    json_escape(team.mode.as_str()),
                    json_escape(&team.status),
                    ledger.notes.len()
                ),
            )
            .and_then(|_| {
                record.session.record_runtime_event(
                    "ultraplan.completed",
                    Actor::Runtime,
                    format!(
                        "{{\"plan_id\":\"{}\",\"goal\":\"{}\",\"evidence_refs\":[\"{}\"]}}",
                        json_escape(&plan.plan_id),
                        json_escape(&plan.goal),
                        json_escape(&plan.evidence_refs[0])
                    ),
                )
            })
            .map_err(|error| format!("{error:?}"))?;
        let plan_approval_id = format!("{}_approval", plan.plan_id);
        record
            .session
            .request_plan_approval(plan_approval_id, Some(plan.goal.clone()))
            .map_err(|error| format!("{error:?}"))?;
        record.plan_mode_active = true;
        remember_session(record, format!("ultraplan completed: {}", plan.plan_id));
        Ok(plan)
    }

    pub fn run_ultrareview_fixture(
        &self,
        session_id: &str,
        target: &str,
    ) -> Result<UltraReviewReport, String> {
        let (team, ledger, report) = build_ultrareview_fixture(target);
        report.validate(&ledger)?;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        if record.handle.model_mode != RuntimeModelMode::DeepSeek {
            return Err("UltraReview v1 requires DeepSeek native mode".to_string());
        }
        record
            .session
            .record_runtime_event(
                "agentteam.completed",
                Actor::Runtime,
                format!(
                    "{{\"team_id\":\"{}\",\"mode\":\"{}\",\"status\":\"{}\",\"evidence_count\":{}}}",
                    json_escape(&team.team_id),
                    json_escape(team.mode.as_str()),
                    json_escape(&team.status),
                    ledger.notes.len()
                ),
            )
            .and_then(|_| {
                record.session.record_runtime_event(
                    "ultrareview.completed",
                    Actor::Runtime,
                    format!(
                        "{{\"report_id\":\"{}\",\"target\":\"{}\",\"verified_findings\":{},\"overall_status\":\"{}\"}}",
                        json_escape(&report.report_id),
                        json_escape(target),
                        report.verified_findings.len(),
                        json_escape(&report.overall_status)
                    ),
                )
            })
            .map_err(|error| format!("{error:?}"))?;
        remember_session(
            record,
            format!("ultrareview completed: {}", report.report_id),
        );
        Ok(report)
    }

    pub fn run_deepseek_agent_loop_with_transport<T: LiveHttpTransport>(
        &self,
        transport: &T,
        session_id: &str,
        prompt: &str,
        endpoint: NativeProviderEndpoint,
        max_iterations: usize,
        max_tool_calls: usize,
    ) -> Result<NativeAgentLoopResult, String> {
        self.reset_interrupt();
        self.run_deepseek_agent_loop_with_transport_inner(
            transport,
            session_id,
            prompt,
            endpoint,
            max_iterations,
            max_tool_calls,
            None,
        )
    }

    pub fn run_deepseek_agent_loop_with_transport_and_event_sink<T: LiveHttpTransport>(
        &self,
        transport: &T,
        session_id: &str,
        prompt: &str,
        endpoint: NativeProviderEndpoint,
        max_iterations: usize,
        max_tool_calls: usize,
        event_sink: &mut dyn FnMut(&str),
    ) -> Result<NativeAgentLoopResult, String> {
        self.run_deepseek_agent_loop_with_transport_inner(
            transport,
            session_id,
            prompt,
            endpoint,
            max_iterations,
            max_tool_calls,
            Some(event_sink),
        )
    }

    fn run_deepseek_agent_loop_with_transport_inner<T: LiveHttpTransport>(
        &self,
        transport: &T,
        session_id: &str,
        prompt: &str,
        endpoint: NativeProviderEndpoint,
        max_iterations: usize,
        max_tool_calls: usize,
        mut event_sink: Option<&mut dyn FnMut(&str)>,
    ) -> Result<NativeAgentLoopResult, String> {
        self.reset_interrupt();
        let turn_id = format!("runtime_live_turn_{}", monotonic_nonce()?);
        let (handle, native_loop_merge_suffix, evidence_directive, error_recovery) = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let record = sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("unknown session_id {session_id}"))?;
            if record.handle.model_mode != RuntimeModelMode::DeepSeek {
                return Err(
                    "run_deepseek_agent_loop_with_transport requires deepseek mode".to_string(),
                );
            }
            // Clear transient failure counters, but keep path corrections across turns so
            // the next model call does not rediscover known missing paths.
            record.repeated_tool_failures.clear();
            record
                .session
                .begin_interactive_turn(&turn_id, "runtime_live_deepseek_loop")
                .map_err(|error| format!("{error:?}"))?;
            let evidence_directive = build_runtime_evidence_directive(record);
            if !evidence_directive.is_empty() {
                record
                    .session
                    .record_runtime_event(
                        "runtime.evidence_ledger.injected",
                        Actor::Runtime,
                        format!(
                            "{{\"session_id\":{},\"turn_id\":{},\"file_count\":{},\"memory_count\":{},\"plan_mode_active\":{},\"pending_native_decision\":{}}}",
                            json_string(session_id),
                            json_string(&turn_id),
                            record.file_state.len(),
                            record.session_memory.len(),
                            record.plan_mode_active,
                            record.pending_native_decision.is_some()
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
            record.plan_mode_active = false;
            let suffix = record.session.event_count() as u64 + 1;
            (
                record.handle.clone(),
                suffix,
                evidence_directive,
                record.error_recovery.clone(),
            )
        };
        let prompt_with_context = self.build_native_prompt_with_runtime_context(
            session_id,
            prompt,
            &evidence_directive,
            1200,
        )?;
        let mut endpoint = endpoint;
        endpoint.live_calls_enabled_by_default = true;
        let request = NativeAgentLoopV2Request {
            project_id: handle.project_id,
            session_id: handle.session_id.clone(),
            task_id: handle.task_id,
            turn_id: Some(turn_id.clone()),
            workspace_root: handle.workspace_root,
            artifact_root: handle.artifact_root,
            endpoint,
            prompt: prompt_with_context,
            max_tokens: deepseek_runtime_max_tokens_for_prompt(prompt),
            max_iterations: effective_live_max_iterations(max_iterations),
            max_tool_calls: effective_live_max_tool_calls(max_tool_calls, prompt),
            tool_exposure: native_agent_tool_exposure_for_route(&TurnRouter::classify(
                prompt, None, 0,
            )),
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: Some(error_recovery),
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        let mut live_ingested_lines = HashSet::new();
        let result = if event_sink.is_some() {
            let mut sink = |line: &str| match self.ingest_native_loop_event_jsonl_line(
                session_id,
                line,
                native_loop_merge_suffix,
            ) {
                Ok(true) => {
                    live_ingested_lines.insert(line.to_string());
                    if let Some(external_sink) = event_sink.as_deref_mut() {
                        external_sink(line);
                    }
                }
                Ok(false) => {
                    live_ingested_lines.insert(line.to_string());
                }
                Err(_) => {
                    if let Some(external_sink) = event_sink.as_deref_mut() {
                        external_sink(line);
                    }
                }
            };
            let interrupt = self.interrupt_handle();
            AgentKernel::for_request(&request).run_turn_with_interrupt(
                transport,
                request,
                Some(&mut sink),
                interrupt.as_ref(),
            )
        } else {
            let interrupt = self.interrupt_handle();
            AgentKernel::for_request(&request).run_turn_with_interrupt(
                transport,
                request,
                None,
                interrupt.as_ref(),
            )
        }?;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        match native_loop_events_for_final_merge(&result.event_jsonl, &live_ingested_lines) {
            Ok(new_events) => {
                record
                    .session
                    .merge_events_with_id_suffix(&new_events, native_loop_merge_suffix);
                for event in &new_events {
                    apply_native_loop_event_side_effect(record, event);
                }
                if let Err(e) =
                    set_session_state_if_changed(&mut record.session, result.final_state)
                {
                    let _ = record.session.record_runtime_event(
                        "runtime.state_update_failed",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"session_id\":\"{}\",\"reason\":\"{}\"}}",
                            json_escape(session_id),
                            json_escape(&trim_for_memory(&format!("{e:?}"), 280))
                        ),
                    );
                }
            }
            Err(error) => {
                let _ = record.session.record_runtime_event(
                    "runtime.eventlog_import_failed",
                    researchcode_kernel::Actor::Runtime,
                    format!(
                        "{{\"session_id\":\"{}\",\"reason\":\"{}\"}}",
                        json_escape(session_id),
                        json_escape(&trim_for_memory(&error, 280))
                    ),
                );
                // Best-effort state update even when event import fails.
                if let Err(e) =
                    set_session_state_if_changed(&mut record.session, result.final_state)
                {
                    let _ = record.session.record_runtime_event(
                        "runtime.state_update_failed",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"session_id\":\"{}\",\"reason\":\"{}\"}}",
                            json_escape(session_id),
                            json_escape(&trim_for_memory(&format!("{e:?}"), 280))
                        ),
                    );
                }
                remember_session(
                    record,
                    format!(
                        "native deepseek loop import fallback kept prior session: {}",
                        trim_for_memory(&error, 120)
                    ),
                );
                return Ok(result);
            }
        }
        record.plan_mode_active = result.final_state == AgentState::WaitingForPlanApproval
            || result.event_jsonl.contains("\"plan.mode_entered\"");
        record.pending_native_decision = runtime_pending_native_decision_from_result(
            session_id,
            &record.session,
            &result,
            "deepseek_native_loop",
        )?;
        if result.final_state == AgentState::Completed {
            record.error_recovery.on_success();
        }
        maybe_apply_html_write_intent_fallback(record, prompt)?;
        remember_session(
            record,
            format!(
                "native deepseek loop status={:?} models={} tools={}",
                result.status, result.model_call_count, result.tool_call_count
            ),
        );
        Ok(result)
    }

    pub fn run_qwen_agent_loop_with_transport<T: LiveHttpTransport>(
        &self,
        transport: &T,
        session_id: &str,
        prompt: &str,
        endpoint: NativeProviderEndpoint,
        max_iterations: usize,
        max_tool_calls: usize,
    ) -> Result<NativeAgentLoopResult, String> {
        self.reset_interrupt();
        self.run_qwen_agent_loop_with_transport_inner(
            transport,
            session_id,
            prompt,
            endpoint,
            max_iterations,
            max_tool_calls,
            None,
        )
    }

    pub fn run_qwen_agent_loop_with_transport_and_event_sink<T: LiveHttpTransport>(
        &self,
        transport: &T,
        session_id: &str,
        prompt: &str,
        endpoint: NativeProviderEndpoint,
        max_iterations: usize,
        max_tool_calls: usize,
        event_sink: &mut dyn FnMut(&str),
    ) -> Result<NativeAgentLoopResult, String> {
        self.run_qwen_agent_loop_with_transport_inner(
            transport,
            session_id,
            prompt,
            endpoint,
            max_iterations,
            max_tool_calls,
            Some(event_sink),
        )
    }

    fn run_qwen_agent_loop_with_transport_inner<T: LiveHttpTransport>(
        &self,
        transport: &T,
        session_id: &str,
        prompt: &str,
        endpoint: NativeProviderEndpoint,
        max_iterations: usize,
        max_tool_calls: usize,
        mut event_sink: Option<&mut dyn FnMut(&str)>,
    ) -> Result<NativeAgentLoopResult, String> {
        self.reset_interrupt();
        let turn_id = format!("runtime_live_turn_{}", monotonic_nonce()?);
        let (handle, native_loop_merge_suffix, evidence_directive, error_recovery) = {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let record = sessions
                .get_mut(session_id)
                .ok_or_else(|| format!("unknown session_id {session_id}"))?;
            if record.handle.model_mode != RuntimeModelMode::Qwen {
                return Err("run_qwen_agent_loop_with_transport requires qwen mode".to_string());
            }
            // Clear cross-turn accumulators that cause false positives
            record.repeated_tool_failures.clear();
            record.path_corrections.clear();
            record
                .session
                .begin_interactive_turn(&turn_id, "runtime_live_qwen_loop")
                .map_err(|error| format!("{error:?}"))?;
            let evidence_directive = build_runtime_evidence_directive(record);
            if !evidence_directive.is_empty() {
                record
                    .session
                    .record_runtime_event(
                        "runtime.evidence_ledger.injected",
                        Actor::Runtime,
                        format!(
                            "{{\"session_id\":{},\"turn_id\":{},\"file_count\":{},\"memory_count\":{},\"plan_mode_active\":{},\"pending_native_decision\":{}}}",
                            json_string(session_id),
                            json_string(&turn_id),
                            record.file_state.len(),
                            record.session_memory.len(),
                            record.plan_mode_active,
                            record.pending_native_decision.is_some()
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
            record.plan_mode_active = false;
            let suffix = record.session.event_count() as u64 + 1;
            (
                record.handle.clone(),
                suffix,
                evidence_directive,
                record.error_recovery.clone(),
            )
        };
        let prompt_with_context = self.build_native_prompt_with_runtime_context(
            session_id,
            prompt,
            &evidence_directive,
            800,
        )?;
        let mut endpoint = endpoint;
        endpoint.live_calls_enabled_by_default = true;
        let request = NativeAgentLoopV2Request {
            project_id: handle.project_id,
            session_id: handle.session_id.clone(),
            task_id: handle.task_id,
            turn_id: Some(turn_id.clone()),
            workspace_root: handle.workspace_root,
            artifact_root: handle.artifact_root,
            endpoint,
            prompt: prompt_with_context,
            max_tokens: qwen_runtime_max_tokens_for_prompt(prompt),
            max_iterations: effective_live_max_iterations(max_iterations),
            max_tool_calls: effective_live_max_tool_calls(max_tool_calls, prompt),
            tool_exposure: native_agent_tool_exposure_for_route(&TurnRouter::classify(
                prompt, None, 0,
            )),
            permission_mode: PermissionMode::Default,
            provided_permission_decisions: Vec::new(),
            deepseek_adaptation: None,
            error_recovery: Some(error_recovery),
            hook_dispatcher: None,
            concurrent_tool_execution: false,
        };
        let mut live_ingested_lines = HashSet::new();
        let result = if event_sink.is_some() {
            let mut sink = |line: &str| match self.ingest_native_loop_event_jsonl_line(
                session_id,
                line,
                native_loop_merge_suffix,
            ) {
                Ok(true) => {
                    live_ingested_lines.insert(line.to_string());
                    if let Some(external_sink) = event_sink.as_deref_mut() {
                        external_sink(line);
                    }
                }
                Ok(false) => {
                    live_ingested_lines.insert(line.to_string());
                }
                Err(_) => {
                    if let Some(external_sink) = event_sink.as_deref_mut() {
                        external_sink(line);
                    }
                }
            };
            let interrupt = self.interrupt_handle();
            AgentKernel::for_request(&request).run_turn_with_interrupt(
                transport,
                request,
                Some(&mut sink),
                interrupt.as_ref(),
            )
        } else {
            let interrupt = self.interrupt_handle();
            AgentKernel::for_request(&request).run_turn_with_interrupt(
                transport,
                request,
                None,
                interrupt.as_ref(),
            )
        }?;
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        match native_loop_events_for_final_merge(&result.event_jsonl, &live_ingested_lines) {
            Ok(new_events) => {
                record
                    .session
                    .merge_events_with_id_suffix(&new_events, native_loop_merge_suffix);
                for event in &new_events {
                    apply_native_loop_event_side_effect(record, event);
                }
                if let Err(e) =
                    set_session_state_if_changed(&mut record.session, result.final_state)
                {
                    let _ = record.session.record_runtime_event(
                        "runtime.state_update_failed",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"session_id\":\"{}\",\"reason\":\"{}\"}}",
                            json_escape(session_id),
                            json_escape(&trim_for_memory(&format!("{e:?}"), 280))
                        ),
                    );
                }
            }
            Err(error) => {
                let _ = record.session.record_runtime_event(
                    "runtime.eventlog_import_failed",
                    researchcode_kernel::Actor::Runtime,
                    format!(
                        "{{\"session_id\":\"{}\",\"reason\":\"{}\"}}",
                        json_escape(session_id),
                        json_escape(&trim_for_memory(&error, 280))
                    ),
                );
                if let Err(e) =
                    set_session_state_if_changed(&mut record.session, result.final_state)
                {
                    let _ = record.session.record_runtime_event(
                        "runtime.state_update_failed",
                        researchcode_kernel::Actor::Runtime,
                        format!(
                            "{{\"session_id\":\"{}\",\"reason\":\"{}\"}}",
                            json_escape(session_id),
                            json_escape(&trim_for_memory(&format!("{e:?}"), 280))
                        ),
                    );
                }
                remember_session(
                    record,
                    format!(
                        "native qwen loop import fallback kept prior session: {}",
                        trim_for_memory(&error, 120)
                    ),
                );
                return Ok(result);
            }
        }
        record.plan_mode_active = result.final_state == AgentState::WaitingForPlanApproval
            || result.event_jsonl.contains("\"plan.mode_entered\"");
        record.pending_native_decision = runtime_pending_native_decision_from_result(
            session_id,
            &record.session,
            &result,
            "qwen_native_loop",
        )?;
        if result.final_state == AgentState::Completed {
            record.error_recovery.on_success();
        }
        maybe_apply_html_write_intent_fallback(record, prompt)?;
        remember_session(
            record,
            format!(
                "native qwen loop status={:?} models={} tools={}",
                result.status, result.model_call_count, result.tool_call_count
            ),
        );
        Ok(result)
    }

    pub fn record_live_model_blocked(
        &self,
        session_id: &str,
        provider: &str,
        gate: &str,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        let call_id = format!("{}_blocked_{}", provider, record.session.event_count() + 1);
        record
            .session
            .record_model_call_blocked(call_id, provider, gate)
            .map_err(|error| format!("{error:?}"))?;
        remember_session(record, format!("{provider} live model blocked: {gate}"));
        Ok(())
    }

    pub fn record_runtime_error(
        &self,
        session_id: &str,
        error_code: &str,
        message: &str,
    ) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions
            .get_mut(session_id)
            .ok_or_else(|| format!("unknown session_id {session_id}"))?;
        record
            .session
            .record_runtime_event(
                "runtime.error",
                Actor::Runtime,
                format!(
                    "{{\"error_code\":\"{}\",\"message\":\"{}\",\"recoverable\":true}}",
                    json_escape(error_code),
                    json_escape(&trim_for_memory(message, 1200))
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        if matches!(
            record.session.state(),
            AgentState::Planning
                | AgentState::RetrievingContext
                | AgentState::Executing
                | AgentState::ApplyingPatch
                | AgentState::RunningCommand
                | AgentState::DiagnosingFailure
        ) {
            record
                .session
                .set_state(AgentState::Failed)
                .map_err(|error| format!("{error:?}"))?;
        }
        remember_session(
            record,
            format!(
                "runtime error {}: {}",
                error_code,
                trim_for_memory(message, 240)
            ),
        );
        Ok(())
    }
}

fn plan_mode_denies_tool(tool_id: &str) -> bool {
    !matches!(
        tool_id,
        "file.read"
            | "file.list_directory"
            | "file.list_tree"
            | "search.ripgrep"
            | "repo.map"
            | "git.status"
            | "todo.write"
            | "context.compact"
            | "plan.write"
            | "plan.exit"
            | "plan.enter"
    )
}

fn tool_error_to_result(
    tool_call_id: &str,
    tool_id: &str,
    error: &crate::tool_execution::ToolExecutionError,
) -> ToolExecutionResult {
    let text = format!("{error:?}");
    let (error_code, recoverable, next_action_hint) = if text.contains("Is a directory") {
        (
            "path_is_directory",
            true,
            "Use file.list_directory/file.list_tree on the directory, then read a concrete file.",
        )
    } else if text.contains("No such file") {
        (
            "path_not_found",
            true,
            "Use repo.map on the nearest existing parent and correct the path.",
        )
    } else if text.contains("SensitivePath") {
        (
            "sensitive_path",
            false,
            "Do not read secrets or protected paths.",
        )
    } else {
        (
            "tool_failed",
            true,
            "Diagnose the failure and choose a different tool or arguments.",
        )
    };
    ToolExecutionResult {
        tool_call_id: tool_call_id.to_string(),
        tool_id: tool_id.to_string(),
        ok: false,
        preview: format!("tool error {error_code}; {next_action_hint}"),
        detail_json: format!(
            "{{\"ok\":false,\"error_code\":\"{}\",\"recoverable\":{},\"raw_error\":\"{}\",\"next_action_hint\":\"{}\",\"artifact_ref\":null}}",
            json_escape(error_code),
            recoverable,
            json_escape(&text),
            json_escape(next_action_hint)
        ),
        exit_code: None,
    }
}

fn record_patch_proposal_validation(
    session: &mut AgentSession,
    workspace_root: &Path,
    tool_call_id: &str,
    args: &ToolExecutionArgs,
) -> Result<(String, String), String> {
    let path = args
        .path
        .as_deref()
        .ok_or_else(|| "patch.apply missing path".to_string())?;
    let old_string = args.old_string.as_deref().unwrap_or_default();
    let base_hash = args.base_hash.as_deref().unwrap_or_default();
    let patch_id = format!("{tool_call_id}_patch");
    session
        .record_patch_proposal_created(&patch_id, path)
        .map_err(|error| format!("{error:?}"))?;
    let current_path = workspace_root.join(path);
    let current_text = fs::read_to_string(&current_path).ok();
    let current_hash = current_text.as_deref().map(stable_text_hash);
    let validation = validate_patch_allowing_protected(PatchCheck {
        path,
        current_text: current_text.as_deref(),
        current_hash: current_hash.as_deref(),
        old_string,
        base_hash,
    });
    session
        .record_patch_proposal_validated(&patch_id, validation.clone())
        .map_err(|error| format!("{error:?}"))?;
    if !matches!(
        validation,
        PatchValidation::Pass | PatchValidation::PassCreate
    ) {
        return Err(format!("patch validation failed: {validation:?}"));
    }
    Ok((patch_id, path.to_string()))
}

fn facade_tool_mode(
    autonomy_mode: AutonomyMode,
    tool_id: &str,
    args: &ToolExecutionArgs,
) -> FacadeToolMode {
    match tool_id {
        "file.read"
        | "file.list_directory"
        | "file.list_tree"
        | "search.ripgrep"
        | "repo.map"
        | "git.status"
        | "research.csv_profile" => FacadeToolMode::Preview,
        "todo.write" | "ask_user" => FacadeToolMode::Preview,
        "shell.command" => {
            let command = args.command.as_deref().unwrap_or_default();
            if command_contains_hard_deny(command) {
                return FacadeToolMode::Blocked("hard-denied shell command".to_string());
            }
            if autonomy_mode == AutonomyMode::FastAuto && is_fast_auto_safe_command(command) {
                FacadeToolMode::FastAutoApply
            } else {
                FacadeToolMode::RequirePermission(PermissionRequestType::Command)
            }
        }
        "patch.apply" | "file.edit" | "file.write" | "file.multi_edit" => {
            let safe_fast_auto_write = match tool_id {
                "file.write" => args.base_hash.is_some() || args.old_string.is_none(),
                _ => args.base_hash.is_some(),
            };
            if autonomy_mode == AutonomyMode::FastAuto && safe_fast_auto_write {
                FacadeToolMode::FastAutoApply
            } else {
                FacadeToolMode::RequirePermission(PermissionRequestType::FileWrite)
            }
        }
        "artifact.export" => {
            FacadeToolMode::RequirePermission(PermissionRequestType::ArtifactExport)
        }
        _ => FacadeToolMode::Blocked(format!("unknown or unsupported tool {tool_id}")),
    }
}

fn record_permission_context_event(
    session: &mut AgentSession,
    permission_id: &str,
    tool_id: &str,
    request_type: &PermissionRequestType,
    args: &ToolExecutionArgs,
) -> Result<(), String> {
    let args_preview = permission_args_preview(args);
    let path_preview = args
        .path
        .as_deref()
        .or(args.root.as_deref())
        .or(args.output_dir.as_deref())
        .or(args.input_csv.as_deref())
        .unwrap_or("");
    let risk_level = permission_risk_level(tool_id, request_type, args);
    session
        .record_runtime_event(
            "permission.context",
            Actor::Runtime,
            format!(
                "{{\"permission_id\":{},\"tool_id\":{},\"request_type\":{},\"args_preview\":{},\"path_preview\":{},\"risk_level\":{}}}",
                json_string(permission_id),
                json_string(tool_id),
                json_string(permission_request_type_to_wire(request_type)),
                json_string(&args_preview),
                json_string(path_preview),
                json_string(risk_level)
            ),
        )
        .map_err(|error| format!("{error:?}"))
}

fn permission_args_preview(args: &ToolExecutionArgs) -> String {
    if let Some(command) = args
        .command
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        return trim_for_memory(command, 320);
    }
    if let Some(path) = args
        .path
        .as_deref()
        .or(args.root.as_deref())
        .or(args.output_dir.as_deref())
        .or(args.input_csv.as_deref())
        .filter(|value| !value.trim().is_empty())
    {
        let action = args
            .query
            .as_deref()
            .or(args.pattern.as_deref())
            .unwrap_or("file operation");
        return trim_for_memory(&format!("{action}: {path}"), 320);
    }
    trim_for_memory(&format!("{args:?}"), 320)
}

fn permission_risk_level(
    tool_id: &str,
    request_type: &PermissionRequestType,
    args: &ToolExecutionArgs,
) -> &'static str {
    let command = args.command.as_deref().unwrap_or_default().to_lowercase();
    if command_contains_hard_deny(&command)
        || command.contains("rm -rf")
        || command.contains("sudo ")
    {
        return "critical";
    }
    if matches!(
        request_type,
        PermissionRequestType::PackageInstall
            | PermissionRequestType::Network
            | PermissionRequestType::ProtectedPath
    ) {
        return "high";
    }
    if tool_id == "shell.command" || matches!(request_type, PermissionRequestType::Command) {
        return "medium";
    }
    "low"
}

fn permission_request_type_to_wire(value: &PermissionRequestType) -> &'static str {
    match value {
        PermissionRequestType::Command => "command",
        PermissionRequestType::FileWrite => "file_write",
        PermissionRequestType::Network => "network",
        PermissionRequestType::PackageInstall => "package_install",
        PermissionRequestType::CloudModel => "cloud_model",
        PermissionRequestType::ProtectedPath => "protected_path",
        PermissionRequestType::ArtifactExport => "artifact_export",
    }
}

fn is_fast_auto_safe_command(command: &str) -> bool {
    let trimmed = command.trim();
    [
        "cargo test",
        "cargo check",
        "python3 -m unittest",
        "rg ",
        "find ",
        "ls",
    ]
    .iter()
    .any(|prefix| trimmed == prefix.trim() || trimmed.starts_with(prefix))
}

fn command_contains_hard_deny(command: &str) -> bool {
    let classification = classify_command_with_reasons(command);
    if matches!(classification.decision, CommandDecision::Deny) {
        return true;
    }
    let lower = command.to_ascii_lowercase();
    lower.contains(" rm ")
        || lower.starts_with("rm ")
        || lower.contains("git reset")
        || lower.contains("git push")
        || lower.contains("curl ")
        || lower.contains("wget ")
        || lower.contains("npm install")
        || lower.contains("pnpm install")
        || lower.contains("pip install")
        || lower.contains(".env")
        || lower.contains("id_rsa")
        || lower.contains("id_ed25519")
        || lower.contains('|')
        || lower.contains('>')
        || lower.contains("$(")
}

fn inject_base_hash_from_last_read(
    record: &RuntimeSessionRecord,
    tool_id: &str,
    args: &mut ToolExecutionArgs,
) {
    if !matches!(tool_id, "file.edit" | "file.multi_edit") || args.base_hash.is_some() {
        return;
    }
    let Some(path) = args.path.as_deref() else {
        return;
    };
    if let Some(state) = record.file_state.get(path) {
        args.base_hash = Some(state.content_hash.clone());
    }
}

fn read_before_write_violation(
    record: &RuntimeSessionRecord,
    tool_id: &str,
    args: &ToolExecutionArgs,
) -> Option<String> {
    if !matches!(tool_id, "file.edit" | "file.multi_edit" | "file.write") {
        return None;
    }
    let path = args.path.as_deref()?;
    let Some(base_hash) = args.base_hash.as_deref() else {
        return if matches!(tool_id, "file.edit" | "file.multi_edit") {
            Some(format!(
                "{tool_id} requires reading {path} in this RuntimeFacade session before writing"
            ))
        } else {
            None
        };
    };
    let Some(state) = record.file_state.get(path) else {
        return Some(format!(
            "{tool_id} requires reading {path} in this RuntimeFacade session before writing"
        ));
    };
    if state.content_hash != base_hash {
        return Some(format!(
            "{tool_id} base_hash does not match last read hash for {path}"
        ));
    }
    None
}

fn update_runtime_tool_state(record: &mut RuntimeSessionRecord, result: &ToolExecutionResult) {
    remember_session(
        record,
        format!(
            "tool {} ok={} preview={}",
            result.tool_id,
            result.ok,
            trim_for_memory(
                &result.preview,
                tool_memory_budget(record.handle.model_mode)
            )
        ),
    );
    if result.ok {
        record.repeated_tool_failures.remove(&format!(
            "{}:{}",
            result.tool_id,
            normalized_result_path(result)
        ));
    }
    if result.tool_id == "file.read" && result.ok {
        if let (Some(path), Some(content_hash)) = (
            extract_json_string_field(&result.detail_json, "path"),
            extract_json_string_field(&result.detail_json, "content_hash"),
        ) {
            let line_start = extract_json_u64_field(&result.detail_json, "line_start");
            let line_end = extract_json_u64_field(&result.detail_json, "line_end");
            let range = line_start
                .zip(line_end)
                .filter(|(start, end)| *start <= *end);
            record
                .file_state
                .entry(path.clone())
                .and_modify(|state| {
                    state.content_hash = content_hash.clone();
                    state.line_start = line_start;
                    state.line_end = line_end;
                    if let Some(range) = range {
                        if !state.read_ranges.contains(&range) {
                            state.read_ranges.push(range);
                        }
                    }
                    if state.read_ranges.len() > 12 {
                        let drain_count = state.read_ranges.len() - 12;
                        state.read_ranges.drain(0..drain_count);
                    }
                })
                .or_insert_with(|| RuntimeFileState {
                    path,
                    content_hash,
                    line_start,
                    line_end,
                    read_ranges: range.into_iter().collect(),
                });
        }
    }
    if matches!(
        result.tool_id.as_str(),
        "file.edit" | "file.multi_edit" | "file.write"
    ) && result.ok
    {
        if let (Some(path), Some(content_hash)) = (
            extract_json_string_field(&result.detail_json, "path"),
            extract_json_string_field(&result.detail_json, "new_hash"),
        ) {
            record
                .file_state
                .entry(path.clone())
                .and_modify(|state| {
                    state.content_hash = content_hash.clone();
                    state.line_start = None;
                    state.line_end = None;
                    state.read_ranges.clear();
                })
                .or_insert_with(|| RuntimeFileState {
                    path,
                    content_hash,
                    line_start: None,
                    line_end: None,
                    read_ranges: Vec::new(),
                });
        }
    }
    if !result.ok {
        let error_code = extract_json_string_field(&result.detail_json, "error_code")
            .unwrap_or_else(|| "tool_failed".to_string());
        let path = normalized_result_path(result);
        let failure_key = format!("{}:{path}:{error_code}", result.tool_id);
        let count = {
            let entry = record
                .repeated_tool_failures
                .entry(failure_key)
                .or_insert(0);
            *entry += 1;
            *entry
        };
        remember_session(
            record,
            format!(
                "failed tool={} error_code={} path={} count={} hint={}",
                result.tool_id,
                error_code,
                path,
                count,
                extract_json_string_field(&result.detail_json, "next_action_hint")
                    .unwrap_or_default()
            ),
        );
        if error_code == "path_is_directory" {
            record.discovered_roots.retain(|existing| existing != &path);
            record.discovered_roots.push(path.clone());
            record.path_corrections.insert(
                path.clone(),
                "use file.list_directory/file.list_tree then read concrete files".to_string(),
            );
        }
        if error_code == "path_not_found" {
            record.path_corrections.insert(
                path.clone(),
                "path does not exist in this workspace; list the nearest existing parent and choose a concrete path from that listing".to_string(),
            );
        }
        if count >= 2 {
            remember_session(
                record,
                format!(
                    "corrective_hint: stop repeating {} on {}; switch strategy to file.list_directory/file.list_tree/path correction",
                    result.tool_id, path
                ),
            );
        }
    }
}

fn normalized_result_path(result: &ToolExecutionResult) -> String {
    extract_json_string_field(&result.detail_json, "path")
        .unwrap_or_else(|| "(no-path)".to_string())
}

fn subagent_tool_writes(tool_id: &str) -> bool {
    matches!(
        tool_id,
        "file.write" | "file.edit" | "file.multi_edit" | "patch.apply"
    )
}

struct SubagentWriteScopeHook {
    write_scope: Vec<String>,
}

impl Hook for SubagentWriteScopeHook {
    fn matches(&self, event: &HookEvent) -> bool {
        matches!(event, HookEvent::PreToolUse { tool_id, .. } if subagent_tool_writes(tool_id))
    }

    fn handle(&self, event: &HookEvent) -> HookDecision {
        let HookEvent::PreToolUse {
            tool_id, args_json, ..
        } = event
        else {
            return HookDecision::Allow;
        };
        let path = serde_json::from_str::<serde_json::Value>(args_json)
            .ok()
            .and_then(|value| {
                value
                    .get("path")
                    .and_then(|path| path.as_str())
                    .map(str::to_string)
            });
        let Some(path) = path else {
            return HookDecision::Deny {
                reason: format!("{tool_id} requires path for subagent write_scope enforcement"),
            };
        };
        if path_is_within_any_scope(&path, &self.write_scope) {
            HookDecision::Allow
        } else {
            HookDecision::Deny {
                reason: format!("path {path} is outside subagent write_scope"),
            }
        }
    }
}

fn path_is_within_any_scope(path: &str, scopes: &[String]) -> bool {
    if scopes.is_empty() {
        return false;
    }
    if normalize_user_supplied_path(path)
        .split('/')
        .any(|part| part == "..")
    {
        return false;
    }
    let normalized_path = normalize_scope_path(path);
    scopes.iter().any(|scope| {
        let normalized_scope = normalize_scope_path(scope);
        normalized_path == normalized_scope
            || normalized_path
                .strip_prefix(&normalized_scope)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn task_dispatch_write_scope_paths(value: Option<&str>) -> Result<Vec<String>, String> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let parsed = serde_json::from_str::<serde_json::Value>(trimmed)
        .map_err(|error| format!("invalid write_scope JSON: {error}"))?;
    let paths = parsed
        .get("paths")
        .or_else(|| {
            if parsed.is_array() {
                Some(&parsed)
            } else {
                None
            }
        })
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(paths)
}

fn normalize_scope_path(path: &str) -> String {
    let mut parts = Vec::new();
    let normalized = normalize_user_supplied_path(path);
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            value => parts.push(value),
        }
    }
    parts.join("/")
}

fn infer_path_from_subagent_message(message: &str) -> Option<String> {
    for token in message.split_whitespace() {
        let cleaned = token
            .trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\'' | '`' | ',' | ';' | ':' | '，' | '。' | '：'
                )
            })
            .to_string();
        if cleaned.starts_with('/') || cleaned.contains('/') {
            return Some(cleaned);
        }
    }
    None
}

fn normalize_user_supplied_path(path: &str) -> String {
    path.replace("\\ ", " ")
        .replace("\\(", "(")
        .replace("\\)", ")")
        .trim_matches('"')
        .trim_matches('\'')
        .to_string()
}

fn infer_search_pattern(message: &str) -> Option<String> {
    if let Some(quoted) = message.split('"').nth(1) {
        if !quoted.trim().is_empty() {
            return Some(quoted.trim().to_string());
        }
    }
    message
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|character: char| {
                matches!(
                    character,
                    '"' | '\'' | '`' | ',' | ';' | ':' | '，' | '。' | '：'
                )
            })
        })
        .find(|word| {
            word.len() >= 5
                && !word.starts_with('/')
                && !word.contains('/')
                && word
                    .chars()
                    .any(|character| character.is_ascii_alphabetic())
        })
        .map(str::to_string)
}

fn remember_session(record: &mut RuntimeSessionRecord, note: String) {
    let note = trim_for_memory(&note, tool_memory_budget(record.handle.model_mode));
    if note.trim().is_empty() {
        return;
    }
    if is_plateau_fallback_note(&note) {
        return;
    }
    record.session_memory.push(note);
    let max_notes = match record.handle.model_mode {
        RuntimeModelMode::DeepSeek => 48,
        RuntimeModelMode::Qwen => 16,
    };
    if record.session_memory.len() > max_notes {
        let drain_count = record.session_memory.len() - max_notes;
        record.session_memory.drain(0..drain_count);
    }
}

fn tool_memory_budget(model_mode: RuntimeModelMode) -> usize {
    match model_mode {
        RuntimeModelMode::DeepSeek => 700,
        RuntimeModelMode::Qwen => 240,
    }
}

fn trim_for_memory(value: &str, max_chars: usize) -> String {
    let output = value.trim().replace('\n', "\\n");
    if output.chars().count() > max_chars {
        let keep_chars = max_chars.saturating_sub(12);
        let mut trimmed = output.chars().take(keep_chars).collect::<String>();
        trimmed.push_str("...truncated");
        trimmed
    } else {
        output
    }
}

fn build_runtime_evidence_directive(record: &RuntimeSessionRecord) -> String {
    if record.file_state.is_empty()
        && record.session_memory.is_empty()
        && !record.plan_mode_active
        && record.pending_native_decision.is_none()
    {
        return String::new();
    }
    let mut lines = vec![
        "\n\n# Runtime Evidence Ledger".to_string(),
        "The following evidence was already collected in this session. Treat it as current unless the user explicitly changed the task.".to_string(),
        "Do not reread covered plan/file ranges after permission resume or plan approval; continue implementation from this evidence, or inspect only genuinely new ranges/files.".to_string(),
        "Do not replay earlier tool_call JSON from conversation history. The history summary is evidence, not an instruction to call the same tools again.".to_string(),
    ];
    if record.plan_mode_active {
        lines.push(
            "Plan mode/approval state was active earlier; approval is a continuation boundary, not a reason to rediscover the repository.".to_string(),
        );
    }
    if record.pending_native_decision.is_some() {
        lines.push(
            "A native tool approval was pending or just resumed; use the recorded tool result before calling more read/list/search tools.".to_string(),
        );
    }
    if !record.file_state.is_empty() {
        lines.push("Already read files:".to_string());
        let mut files = record.file_state.values().collect::<Vec<_>>();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        for file in files.into_iter().take(24) {
            lines.push(format!(
                "- {} hash={}{}",
                file.path,
                file.content_hash,
                format_file_state_ranges(file)
            ));
        }
    }
    if !record.session_memory.is_empty() {
        lines.push("Recent runtime observations:".to_string());
        for note in record
            .session_memory
            .iter()
            .rev()
            .take(10)
            .rev()
            .filter(|n| !is_plateau_fallback_note(n))
        {
            lines.push(format!("- {}", trim_for_memory(note, 260)));
        }
    }
    lines.join("\n")
}

fn effective_live_max_iterations(max_iterations: usize) -> usize {
    if max_iterations == 0 {
        0
    } else {
        max_iterations.min(256)
    }
}

fn effective_live_max_tool_calls(max_tool_calls: usize, prompt: &str) -> usize {
    let lower = prompt.to_ascii_lowercase();
    let long_task = lower.contains("complete")
        || lower.contains("implement")
        || lower.contains("fix")
        || lower.contains("repair")
        || lower.contains("continue")
        || lower.contains("finish")
        || lower.contains("build")
        || prompt.contains("继续")
        || prompt.contains("完成")
        || prompt.contains("修复")
        || prompt.contains("实现")
        || prompt.contains("编码")
        || prompt.contains("开始写")
        || prompt.contains("开写")
        || prompt.contains("写")
        || prompt.contains("创建")
        || prompt.contains("修改")
        || prompt.contains("编辑")
        || prompt.contains("落地")
        || prompt.contains("全部");
    if max_tool_calls == 0 {
        0
    } else if long_task {
        max_tool_calls.max(64).min(256)
    } else {
        max_tool_calls.min(256)
    }
}

fn json_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn json_string(value: &str) -> String {
    format!("\"{}\"", json_escape(value))
}

fn opt_json_string(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_string())
}

fn native_loop_events_for_final_merge(
    event_jsonl: &str,
    already_ingested_lines: &HashSet<String>,
) -> Result<Vec<researchcode_kernel::KernelEvent>, String> {
    if already_ingested_lines.is_empty() {
        return EventLog::import_jsonl(event_jsonl)
            .map_err(|error| format!("{error:?}"))
            .and_then(|event_log| {
                AgentSession::resume_from_event_log(event_log).map_err(|error| format!("{error:?}"))
            })
            .map(|session| {
                session
                    .event_log()
                    .iter()
                    .filter(|event| native_loop_event_visible_to_facade(event))
                    .cloned()
                    .collect()
            });
    }

    let mut events = Vec::new();
    for line in event_jsonl.lines() {
        if line.trim().is_empty() || already_ingested_lines.contains(line) {
            continue;
        }
        let event = EventLog::parse_jsonl_event(line).map_err(|error| format!("{error:?}"))?;
        if native_loop_event_visible_to_facade(&event) {
            events.push(event);
        }
    }
    Ok(events)
}

fn native_loop_event_visible_to_facade(event: &researchcode_kernel::KernelEvent) -> bool {
    match event.event_type.as_str() {
        "session.created"
        | "session.state_changed"
        | "session.turn_started"
        | "session.forced_transition" => false,
        "model.stream_delta"
            if event.payload_json.contains("\"provider\":\"user\"")
                && event.payload_json.contains("\"delta_kind\":\"input\"") =>
        {
            false
        }
        _ => true,
    }
}

fn apply_native_loop_event_side_effect(record: &mut RuntimeSessionRecord, event: &KernelEvent) {
    match event.event_type.as_str() {
        "tool.call_completed" => {
            if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.payload_json) {
                if let (Some(tool_call_id), Some(ok)) = (
                    payload.get("tool_call_id").and_then(|value| value.as_str()),
                    payload.get("ok").and_then(|value| value.as_bool()),
                ) {
                    record
                        .native_tool_completion
                        .insert(tool_call_id.to_string(), ok);
                }
            }
        }
        "tool.result_recorded" => {
            if let Some(result) = tool_execution_result_from_recorded_event(record, event) {
                update_runtime_tool_state(record, &result);
            }
        }
        "plan.mode_entered" => {
            record.plan_mode_active = true;
            remember_session(
                record,
                "plan mode entered; preserve already-read evidence after approval".to_string(),
            );
        }
        "plan.approval_decided" => {
            if event.payload_json.contains("\"decision\":\"approve\"")
                || event.payload_json.contains("\"decision\":\"Approve\"")
            {
                remember_session(
                    record,
                    "plan approved; continue from existing evidence without rereading covered plan ranges"
                        .to_string(),
                );
            }
        }
        "runtime.permission_resume.tool_executed" => {
            remember_session(
                record,
                "permission resume executed the pending native tool; continue with recorded tool result"
                    .to_string(),
            );
        }
        _ => {}
    }
}

fn tool_execution_result_from_recorded_event(
    record: &RuntimeSessionRecord,
    event: &KernelEvent,
) -> Option<ToolExecutionResult> {
    let payload = serde_json::from_str::<serde_json::Value>(&event.payload_json).ok()?;
    let tool_call_id = payload.get("tool_call_id")?.as_str()?.to_string();
    let tool_id = payload.get("tool_id")?.as_str()?.to_string();
    let content_hash = payload.get("content_hash")?.as_str()?;
    let artifact_path = artifact_path_for_content_hash(&record.handle.artifact_root, content_hash)?;
    let artifact_json = fs::read_to_string(artifact_path).ok()?;
    let parsed_artifact = serde_json::from_str::<serde_json::Value>(&artifact_json).ok();
    let preview = parsed_artifact
        .as_ref()
        .and_then(|artifact| artifact.get("preview").and_then(|value| value.as_str()))
        .map(str::to_string)
        .or_else(|| extract_json_string_field(&artifact_json, "preview"))
        .or_else(|| {
            payload
                .get("preview")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    let detail_json = parsed_artifact
        .as_ref()
        .and_then(|artifact| artifact.get("detail").and_then(|value| value.as_str()))
        .map(str::to_string)
        .or_else(|| extract_tool_result_detail_json(&artifact_json))
        .unwrap_or_else(|| "{}".to_string());
    let ok = parsed_artifact
        .as_ref()
        .and_then(|artifact| artifact.get("ok").and_then(|value| value.as_bool()))
        .or_else(|| extract_json_bool_field(&artifact_json, "ok"))
        .or_else(|| record.native_tool_completion.get(&tool_call_id).copied())
        .unwrap_or(!preview.to_ascii_lowercase().contains("tool error"));
    Some(ToolExecutionResult {
        tool_call_id,
        tool_id,
        ok,
        preview,
        detail_json,
        exit_code: None,
    })
}

fn artifact_path_for_content_hash(root: &Path, content_hash: &str) -> Option<PathBuf> {
    if content_hash.is_empty() {
        return None;
    }
    let shard = content_hash.get(0..2).unwrap_or("00");
    Some(root.join("sha256").join(shard).join(content_hash))
}

fn set_session_state_if_changed(
    session: &mut AgentSession,
    next: AgentState,
) -> Result<(), crate::session::SessionError> {
    if session.state() == next {
        Ok(())
    } else {
        session.set_state(next)
    }
}

fn infer_permission_tool_hint(
    record: &RuntimeSessionRecord,
    permission_id: &str,
) -> (Option<String>, Option<String>, Option<String>) {
    let mut tool_id = None;
    let events = record
        .session
        .event_log()
        .iter()
        .cloned()
        .collect::<Vec<_>>();
    for event in events.iter().rev() {
        if event.event_type != "permission.requested" {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.payload_json) else {
            continue;
        };
        if payload
            .get("permission_id")
            .and_then(|value| value.as_str())
            != Some(permission_id)
        {
            continue;
        }
        tool_id = payload
            .get("tool_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
        break;
    }

    let tool_call_id = permission_id
        .strip_suffix("_fast_auto_permission")
        .or_else(|| permission_id.strip_suffix("_permission"))
        .map(str::to_string)
        .or_else(|| {
            permission_id
                .strip_prefix("native_loop_command_perm_")
                .map(|index| format!("native_loop_tool_{index}"))
        })
        .or_else(|| {
            permission_id
                .strip_prefix("native_loop_patch_perm_")
                .map(|index| format!("native_loop_tool_{index}"))
        });

    let mut provider_tool_call_id = None;
    if let Some(ref wanted_tool_call_id) = tool_call_id {
        for event in events.iter().rev() {
            if !matches!(
                event.event_type.as_str(),
                "tool.call.assembled" | "tool.call_requested"
            ) {
                continue;
            }
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.payload_json) else {
                continue;
            };
            if payload.get("tool_call_id").and_then(|value| value.as_str())
                != Some(wanted_tool_call_id.as_str())
            {
                continue;
            }
            if tool_id.is_none() {
                tool_id = payload
                    .get("tool_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
            }
            provider_tool_call_id = payload
                .get("provider_tool_call_id")
                .and_then(|value| value.as_str())
                .map(str::to_string);
            break;
        }
    }

    (tool_call_id, provider_tool_call_id, tool_id)
}

fn runtime_pending_native_decision_from_result(
    session_id: &str,
    session: &AgentSession,
    result: &NativeAgentLoopResult,
    resume_strategy: &str,
) -> Result<Option<RuntimePendingNativeDecision>, String> {
    let Some(pending_tool) = result.pending_tool.clone() else {
        return Ok(None);
    };
    let (tool_call_id, provider_tool_call_id) =
        infer_pending_native_tool_identity_from_session(session, &pending_tool);
    Ok(Some(RuntimePendingNativeDecision {
        session_id: session_id.to_string(),
        turn_id: session.current_turn_id().map(str::to_string),
        permission_id: pending_tool.permission_id.clone(),
        provider_tool_call_id,
        tool_call_id,
        tool_id: pending_tool.tool_id.clone(),
        args: pending_tool.args.clone(),
        blocked_event_jsonl: result.event_jsonl.clone(),
        resume_strategy: resume_strategy.to_string(),
        created_timestamp: monotonic_nonce()?.to_string(),
        pending_tool,
    }))
}

fn infer_pending_native_tool_identity_from_session(
    session: &AgentSession,
    pending_tool: &PendingNativeToolExecution,
) -> (String, Option<String>) {
    let events = session.event_log().iter().cloned().collect::<Vec<_>>();
    for (permission_index, event) in events.iter().enumerate().rev() {
        if event.event_type != "permission.requested" {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.payload_json) else {
            continue;
        };
        if payload
            .get("permission_id")
            .and_then(|value| value.as_str())
            != Some(pending_tool.permission_id.as_str())
        {
            continue;
        }
        for prior in events[..permission_index].iter().rev() {
            if prior.event_type != "tool.call_requested" {
                continue;
            }
            let Ok(payload) = serde_json::from_str::<serde_json::Value>(&prior.payload_json) else {
                continue;
            };
            if payload.get("tool_id").and_then(|value| value.as_str())
                != Some(pending_tool.tool_id.as_str())
            {
                continue;
            }
            if let Some(tool_call_id) = payload.get("tool_call_id").and_then(|value| value.as_str())
            {
                let provider_tool_call_id = payload
                    .get("provider_tool_call_id")
                    .and_then(|value| value.as_str())
                    .map(str::to_string);
                return (tool_call_id.to_string(), provider_tool_call_id);
            }
        }
    }
    (
        pending_tool.tool_call_id.clone(),
        infer_provider_tool_call_id_from_session(session, &pending_tool.tool_call_id),
    )
}

fn infer_provider_tool_call_id_from_session(
    session: &AgentSession,
    tool_call_id: &str,
) -> Option<String> {
    let events = session.event_log().iter().cloned().collect::<Vec<_>>();
    for event in events.iter().rev() {
        if !matches!(
            event.event_type.as_str(),
            "tool.call.assembled" | "tool.call_requested"
        ) {
            continue;
        }
        let Ok(payload) = serde_json::from_str::<serde_json::Value>(&event.payload_json) else {
            continue;
        };
        if payload.get("tool_call_id").and_then(|value| value.as_str()) != Some(tool_call_id) {
            continue;
        }
        return payload
            .get("provider_tool_call_id")
            .and_then(|value| value.as_str())
            .map(str::to_string);
    }
    None
}

fn extract_json_string_field(input: &str, key: &str) -> Option<String> {
    let marker = format!("\"{key}\":\"");
    let start = input.find(&marker)? + marker.len();
    let rest = &input[start..];
    let mut output = String::new();
    let mut escaped = false;
    for ch in rest.chars() {
        if escaped {
            output.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '"' {
            return Some(output);
        }
        output.push(ch);
    }
    None
}

fn extract_json_bool_field(input: &str, key: &str) -> Option<bool> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    if rest.starts_with("true") {
        Some(true)
    } else if rest.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

fn extract_tool_result_detail_json(input: &str) -> Option<String> {
    let marker = "\"detail\":";
    let start = input.find(marker)? + marker.len();
    let rest = &input[start..];
    let end = rest
        .find(",\"privacy_class\"")
        .or_else(|| rest.find(",\"privacy\""))
        .unwrap_or(rest.len());
    let fragment = rest[..end].trim();
    if fragment.is_empty() || fragment == "null" {
        return None;
    }
    if fragment.starts_with('"') && fragment.ends_with('"') {
        return serde_json::from_str::<String>(fragment).ok();
    }
    Some(unescape_json_fragment(fragment))
}

fn unescape_json_fragment(fragment: &str) -> String {
    let mut output = String::new();
    let mut escaped = false;
    for ch in fragment.chars() {
        if escaped {
            match ch {
                'n' => output.push('\n'),
                'r' => output.push('\r'),
                't' => output.push('\t'),
                '"' => output.push('"'),
                '\\' => output.push('\\'),
                other => output.push(other),
            }
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else {
            output.push(ch);
        }
    }
    if escaped {
        output.push('\\');
    }
    output
}

fn extract_json_u64_field(input: &str, key: &str) -> Option<u64> {
    let marker = format!("\"{key}\":");
    let start = input.find(&marker)? + marker.len();
    let rest = input[start..].trim_start();
    let digits = rest
        .chars()
        .take_while(|ch| ch.is_ascii_digit())
        .collect::<String>();
    digits.parse::<u64>().ok()
}

fn extract_session_id_from_jsonl(jsonl: &str) -> Option<String> {
    for line in jsonl.lines() {
        let marker = "\"session_id\":\"";
        let start = line.find(marker)? + marker.len();
        let rest = &line[start..];
        let end = rest.find('"')?;
        return Some(rest[..end].to_string());
    }
    None
}

fn deepseek_runtime_max_tokens_for_prompt(prompt: &str) -> u64 {
    let lowered = prompt.to_ascii_lowercase();
    let wants_generation = deepseek_runtime_prompt_wants_generation(&lowered);
    let wants_deep_analysis = [
        "深度",
        "分析",
        "解析",
        "代码库",
        "repo",
        "repository",
        "ultraplan",
        "ultrareview",
        "review",
    ]
    .iter()
    .any(|needle| lowered.contains(needle));
    if wants_deep_analysis {
        20_000
    } else if wants_generation {
        16_384
    } else {
        8_192
    }
}

fn qwen_runtime_max_tokens_for_prompt(prompt: &str) -> u64 {
    let lowered = prompt.to_ascii_lowercase();
    let wants_generation = deepseek_runtime_prompt_wants_generation(&lowered);
    let wants_deep_analysis = ["深度", "分析", "解析", "repo", "repository", "review"]
        .iter()
        .any(|needle| lowered.contains(needle));
    if wants_deep_analysis {
        12_288
    } else if wants_generation {
        8_192
    } else {
        4_096
    }
}

fn deepseek_runtime_prompt_wants_generation(lowered_prompt: &str) -> bool {
    [
        "html",
        "css",
        "javascript",
        "js",
        "小程序",
        "网页",
        "页面",
        "生成",
        "创建",
        "写个",
        "写入",
        "写进",
        "直接写",
        "保存",
        "新建",
        "文件夹",
        "文件",
        "create file",
        "write file",
        "save file",
        "make file",
        "实现",
        "code",
        "app",
        "tool",
    ]
    .iter()
    .any(|needle| lowered_prompt.contains(needle))
}

fn maybe_apply_html_write_intent_fallback(
    record: &mut RuntimeSessionRecord,
    prompt: &str,
) -> Result<(), String> {
    if std::env::var("RESEARCHCODE_ENABLE_WRITE_INTENT_FALLBACK")
        .ok()
        .as_deref()
        != Some("1")
    {
        return Ok(());
    }
    if !runtime_prompt_requires_html_write(prompt) {
        return Ok(());
    }
    if session_has_successful_file_write(&record.session) {
        return Ok(());
    }
    let path = next_generated_html_path(&record.handle.workspace_root);
    let content = fallback_html_small_program();
    let tool_call_id = format!(
        "runtime_write_intent_fallback_{}",
        record.session.event_count() + 1
    );
    record
        .session
        .record_runtime_event(
            "runtime.write_intent_fallback",
            Actor::Runtime,
            format!(
                "{{\"tool_call_id\":\"{}\",\"tool_id\":\"file.write\",\"path\":\"{}\",\"reason\":\"explicit_html_write_without_model_tool_call\"}}",
                json_escape(&tool_call_id),
                json_escape(&path)
            ),
        )
        .and_then(|_| record.session.record_tool_call_requested(&tool_call_id, "file.write"))
        .and_then(|_| {
            record.session.request_permission(
                format!("{tool_call_id}_fast_auto_permission"),
                PermissionRequestType::FileWrite,
                Some("file.write".to_string()),
            )
        })
        .and_then(|_| {
            record
                .session
                .decide_permission(PermissionDecisionKind::AllowProjectRule)
        })
        .map_err(|error| format!("{error:?}"))?;
    let result = execute_tool(&ToolExecutionRequest {
        workspace_root: record.handle.workspace_root.clone(),
        tool_call_id: tool_call_id.clone(),
        tool_id: "file.write".to_string(),
        mode: ToolExecutionMode::ApplyWithPermission {
            permission_decision: Some(PermissionDecisionKind::AllowOnce),
        },
        args: ToolExecutionArgs {
            path: Some(path.clone()),
            content: Some(content),
            ..ToolExecutionArgs::default()
        },
    })
    .map_err(|error| format!("{error:?}"))?;
    record
        .session
        .record_tool_call_completed(&result.tool_call_id, &result.tool_id, result.ok)
        .and_then(|_| {
            record.session.record_tool_result_artifact(
                &result.tool_call_id,
                &result.tool_id,
                format!("artifact_{}", result.tool_call_id),
                stable_text_hash(&result.detail_json),
                &result.preview,
            )
        })
        .map_err(|error| format!("{error:?}"))?;
    update_runtime_tool_state(record, &result);
    set_session_state_if_changed(&mut record.session, AgentState::Completed)
        .map_err(|error| format!("{error:?}"))?;
    Ok(())
}

fn session_has_successful_file_write(session: &AgentSession) -> bool {
    session.export_events_jsonl().lines().any(|line| {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            return false;
        };
        value
            .get("event_type")
            .and_then(|value| value.as_str())
            .is_some_and(|event_type| event_type == "tool.call_completed")
            && value
                .get("payload")
                .and_then(|payload| payload.get("tool_id"))
                .and_then(|value| value.as_str())
                .is_some_and(|tool_id| tool_id == "file.write")
            && value
                .get("payload")
                .and_then(|payload| payload.get("ok"))
                .and_then(|value| value.as_bool())
                .unwrap_or(false)
    })
}

fn runtime_prompt_requires_html_write(prompt: &str) -> bool {
    let lowered = prompt.to_ascii_lowercase();
    lowered.contains("html")
        && [
            "写入",
            "写进",
            "保存",
            "新建",
            "文件夹",
            "文件",
            "write",
            "save",
            "create",
        ]
        .iter()
        .any(|needle| lowered.contains(needle))
}

fn next_generated_html_path(workspace_root: &Path) -> String {
    let base = "generated_app.html";
    if !workspace_root.join(base).exists() {
        return base.to_string();
    }
    for index in 2..100 {
        let candidate = format!("generated_app_{index}.html");
        if !workspace_root.join(&candidate).exists() {
            return candidate;
        }
    }
    format!("generated_app_{}.html", monotonic_nonce().unwrap_or(0))
}

fn fallback_html_small_program() -> String {
    [
        "<!DOCTYPE html>",
        "<html lang=\"en\">",
        "<head>",
        "  <meta charset=\"UTF-8\">",
        "  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">",
        "  <title>Mini Counter App</title>",
        "  <style>",
        "    body { margin: 0; min-height: 100vh; display: grid; place-items: center; font-family: Arial, sans-serif; background: #f4f7fb; }",
        "    main { width: min(420px, 92vw); padding: 28px; border: 1px solid #d8e0ea; border-radius: 8px; background: white; box-shadow: 0 12px 30px rgba(20, 30, 50, .08); }",
        "    h1 { margin: 0 0 8px; font-size: 24px; }",
        "    p { color: #536070; line-height: 1.5; }",
        "    .count { font-size: 56px; font-weight: 700; margin: 20px 0; color: #1f6feb; }",
        "    button { border: 0; border-radius: 6px; padding: 10px 14px; margin-right: 8px; cursor: pointer; background: #1f6feb; color: white; }",
        "    button.secondary { background: #e8edf5; color: #243044; }",
        "  </style>",
        "</head>",
        "<body>",
        "  <main>",
        "    <h1>Mini Counter</h1>",
        "    <p>A tiny HTML program written by the runtime file tool.</p>",
        "    <div id=\"count\" class=\"count\">0</div>",
        "    <button id=\"add\">Add one</button>",
        "    <button id=\"reset\" class=\"secondary\">Reset</button>",
        "  </main>",
        "  <script>",
        "    let value = 0;",
        "    const count = document.getElementById('count');",
        "    document.getElementById('add').onclick = () => { count.textContent = String(++value); };",
        "    document.getElementById('reset').onclick = () => { value = 0; count.textContent = '0'; };",
        "  </script>",
        "</body>",
        "</html>",
    ]
    .join("\n")
}

fn monotonic_nonce() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagent::SubagentType;

    #[test]
    fn facade_starts_session_and_exports_events() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-session");
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        facade
            .submit_user_message(&handle.session_id, "hello runtime facade")
            .unwrap();
        let snapshot = facade.get_session_snapshot(&handle.session_id).unwrap();
        assert_eq!(snapshot.state, AgentState::Executing);
        assert_eq!(snapshot.autonomy_mode, AutonomyMode::FastAuto);
        assert!(snapshot.event_count >= 4);
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("session.created"));
    }

    #[test]
    fn facade_cancel_session_cancels_active_turn_and_is_terminal_idempotent() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-cancel-session");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();

        assert_eq!(
            facade
                .get_session_snapshot(&handle.session_id)
                .unwrap()
                .state,
            AgentState::Executing
        );
        facade.cancel_session(&handle.session_id).unwrap();
        assert_eq!(
            facade
                .get_session_snapshot(&handle.session_id)
                .unwrap()
                .state,
            AgentState::Cancelled
        );

        facade.cancel_session(&handle.session_id).unwrap();
        assert_eq!(
            facade
                .get_session_snapshot(&handle.session_id)
                .unwrap()
                .state,
            AgentState::Cancelled
        );
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("runtime.turn_cancel_requested"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn subagent_uses_isolated_child_event_log_and_parent_summary() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-subagent-isolated");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "hello child\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let request = SubagentRequest::readonly(
            &handle.session_id,
            SubagentType::Explorer,
            "inspect README",
            NativeModelFamily::DeepSeek,
        );
        let subagent = facade.spawn_subagent(&handle.session_id, request).unwrap();
        let summary = facade
            .run_subagent_task(&subagent.subagent_id, "inspect README.md")
            .unwrap();

        let parent_events = facade
            .stream_agent_events(&handle.session_id)
            .unwrap()
            .jsonl;
        let child_events = facade
            .stream_subagent_events(&subagent.subagent_id)
            .unwrap()
            .jsonl;
        assert!(parent_events.contains("subagent.spawned"));
        assert!(parent_events.contains("subagent.summary_recorded"));
        assert!(!parent_events.contains("subagent.tool_completed"));
        assert!(child_events.contains("subagent.child_created"));
        assert!(child_events.contains("subagent.message_received"));
        assert!(child_events.contains("subagent.tool_completed"));
        assert!(child_events.contains("subagent.completed"));
        assert!(summary
            .evidence_refs
            .iter()
            .any(|item| item.contains("subagent:")));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn cancelled_subagent_cannot_be_run() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-subagent-cancel");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let request = SubagentRequest::readonly(
            &handle.session_id,
            SubagentType::Explorer,
            "inspect only",
            NativeModelFamily::DeepSeek,
        );
        let subagent = facade.spawn_subagent(&handle.session_id, request).unwrap();
        let cancelled = facade.cancel_subagent(&subagent.subagent_id).unwrap();
        assert_eq!(cancelled.status, SubagentStatus::Cancelled);
        let err = facade
            .run_subagent_task(&subagent.subagent_id, "inspect README.md")
            .unwrap_err();
        assert!(err.contains("terminal"));
        let child_events = facade
            .stream_subagent_events(&subagent.subagent_id)
            .unwrap()
            .jsonl;
        assert!(child_events.contains("subagent.cancelled"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn worker_subagent_write_scope_blocks_out_of_scope_writes() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-subagent-write-scope");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let request = SubagentRequest {
            agent_type: SubagentType::Worker,
            task: "write scoped file".to_string(),
            model_family: NativeModelFamily::DeepSeek,
            tool_allowlist: vec!["file.write".to_string()],
            write_scope: vec!["src".to_string()],
            worktree_required: true,
            worktree_ready: true,
            context_pack: crate::subagent::ContextPack::new(&handle.session_id, "worker scope"),
        };
        let subagent = facade.spawn_subagent(&handle.session_id, request).unwrap();
        let ok = facade
            .execute_subagent_tool(
                &subagent.subagent_id,
                "worker_write_ok",
                "file.write",
                ToolExecutionArgs {
                    path: Some("src/generated.rs".to_string()),
                    content: Some("pub const OK: bool = true;\n".to_string()),
                    ..ToolExecutionArgs::default()
                },
                Some(PermissionDecisionKind::AllowOnce),
            )
            .unwrap();
        assert!(ok.ok);
        let blocked = facade
            .execute_subagent_tool(
                &subagent.subagent_id,
                "worker_write_blocked",
                "file.write",
                ToolExecutionArgs {
                    path: Some("outside.rs".to_string()),
                    content: Some("pub const BAD: bool = true;\n".to_string()),
                    ..ToolExecutionArgs::default()
                },
                Some(PermissionDecisionKind::AllowOnce),
            )
            .unwrap_err();
        assert!(blocked.contains("outside write_scope"));
        assert!(root.join("src/generated.rs").exists());
        assert!(!root.join("outside.rs").exists());
        let child_events = facade
            .stream_subagent_events(&subagent.subagent_id)
            .unwrap()
            .jsonl;
        assert!(child_events.contains("subagent.tool_completed"));
        assert!(child_events.contains("subagent.tool_blocked"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn readonly_subagent_can_run_llm_child_loop_with_isolated_events() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-subagent-llm");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "child loop fixture\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let request = SubagentRequest::readonly(
            &handle.session_id,
            SubagentType::Explorer,
            "inspect README",
            NativeModelFamily::DeepSeek,
        );
        let subagent = facade.spawn_subagent(&handle.session_id, request).unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Child inspected README."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let summary = facade
            .run_subagent_model_task_with_transport(
                &transport,
                &subagent.subagent_id,
                "inspect README.md",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
            )
            .unwrap();

        assert_eq!(summary.status, SubagentStatus::Completed);
        let parent_events = facade
            .stream_agent_events(&handle.session_id)
            .unwrap()
            .jsonl;
        let child_events = facade
            .stream_subagent_events(&subagent.subagent_id)
            .unwrap()
            .jsonl;
        assert!(parent_events.contains("subagent.summary_recorded"));
        assert!(!parent_events.contains("subagent.model_turn_started"));
        assert!(child_events.contains("subagent.model_turn_started"));
        assert!(child_events.contains("model.call_started"));
        assert!(child_events.contains("subagent.model_turn_completed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn task_dispatch_facade_runs_readonly_llm_subagent() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-task-dispatch-llm");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "task dispatch fixture\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Task dispatch child complete."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let summary = facade
            .run_task_dispatch_with_transport(
                &transport,
                &handle.session_id,
                "dispatch_llm_1",
                ToolExecutionArgs {
                    content: Some("inspect README.md".to_string()),
                    model_role: Some("reviewer".to_string()),
                    ..ToolExecutionArgs::default()
                },
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
            )
            .unwrap();

        assert_eq!(summary.status, SubagentStatus::Completed);
        let parent_events = facade
            .stream_agent_events(&handle.session_id)
            .unwrap()
            .jsonl;
        assert!(parent_events.contains("tool.call_requested"));
        assert!(parent_events.contains("dispatch_llm_1"));
        assert!(parent_events.contains("subagent.summary_recorded"));
        assert!(!parent_events.contains("subagent.model_turn_started"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn task_dispatch_facade_runs_scoped_worker_llm_subagent() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-task-dispatch-worker");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"src/generated.txt\",\"content\":\"worker wrote scoped file\\n\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Worker completed scoped edit."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let summary = facade
            .run_task_dispatch_with_transport(
                &transport,
                &handle.session_id,
                "dispatch_worker_scoped",
                ToolExecutionArgs {
                    content: Some("create src/generated.txt".to_string()),
                    model_role: Some("executor".to_string()),
                    write_scope_json: Some(r#"{"paths":["src"]}"#.to_string()),
                    ..ToolExecutionArgs::default()
                },
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
            )
            .unwrap();

        assert_eq!(summary.status, SubagentStatus::Completed);
        assert_eq!(
            fs::read_to_string(root.join("src/generated.txt")).unwrap(),
            "worker wrote scoped file\n"
        );
        let parent_events = facade
            .stream_agent_events(&handle.session_id)
            .unwrap()
            .jsonl;
        assert!(parent_events.contains("dispatch_worker_scoped"));
        assert!(parent_events.contains("subagent.summary_recorded"));
        assert!(!parent_events.contains("subagent.model_turn_started"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn task_dispatch_worker_llm_subagent_blocks_out_of_scope_write() {
        let root =
            std::env::temp_dir().join("researchcode-runtime-facade-task-dispatch-worker-blocked");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_write","name":"file_write","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"outside.txt\",\"content\":\"bad\\n\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Cannot write outside the assigned scope."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let result = facade.run_task_dispatch_with_transport(
            &transport,
            &handle.session_id,
            "dispatch_worker_blocked",
            ToolExecutionArgs {
                content: Some("write outside.txt".to_string()),
                model_role: Some("executor".to_string()),
                write_scope_json: Some(r#"{"paths":["src"]}"#.to_string()),
                ..ToolExecutionArgs::default()
            },
            NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
        );

        assert!(!root.join("outside.txt").exists());
        let parent_events = facade
            .stream_agent_events(&handle.session_id)
            .unwrap()
            .jsonl;
        assert!(parent_events.contains("dispatch_worker_blocked"));
        match result {
            Ok(summary) => assert!(matches!(
                summary.status,
                SubagentStatus::Completed | SubagentStatus::Failed
            )),
            Err(error) => assert!(
                error.contains("write_scope") || error.contains("scripted transport"),
                "unexpected task.dispatch worker error: {error}"
            ),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_runtime_error_moves_active_turn_to_failed() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-runtime-error");
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::Qwen,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        facade
            .submit_user_message(&handle.session_id, "hello runtime facade")
            .unwrap();
        facade
            .record_runtime_error(
                &handle.session_id,
                "runtime_turn_failed",
                "sidecar transport refused",
            )
            .unwrap();
        let snapshot = facade.get_session_snapshot(&handle.session_id).unwrap();
        assert_eq!(snapshot.state, AgentState::Failed);
        let events = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(events.jsonl.contains("\"runtime.error\""));
        assert!(events.jsonl.contains("\"to_state\":\"Failed\""));
        facade
            .submit_user_message(&handle.session_id, "second turn")
            .unwrap();
        let snapshot = facade.get_session_snapshot(&handle.session_id).unwrap();
        assert_eq!(snapshot.state, AgentState::Executing);
    }

    #[test]
    fn facade_streams_incremental_event_deltas_by_cursor() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-event-delta");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let first = facade
            .stream_agent_events_since(&handle.session_id, 0, Some(2))
            .unwrap();
        assert_eq!(first.from_cursor, 0);
        assert_eq!(first.next_cursor, 2);
        assert!(first.has_more);
        assert_eq!(first.jsonl.lines().count(), 2);
        facade
            .submit_user_message(&handle.session_id, "hello delta stream")
            .unwrap();
        let second = facade
            .stream_agent_events_since(&handle.session_id, first.next_cursor, None)
            .unwrap();
        assert_eq!(second.from_cursor, first.next_cursor);
        assert!(second.next_cursor > second.from_cursor);
        assert!(second.jsonl.contains("hello delta stream"));
        assert!(!second.has_more);
        let resynced = facade
            .stream_agent_events_since(&handle.session_id, second.next_cursor + 1, None)
            .unwrap();
        assert_eq!(resynced.from_cursor, second.next_cursor);
        assert_eq!(resynced.next_cursor, second.next_cursor);
        assert!(resynced.events.is_empty());
        assert!(resynced.jsonl.is_empty());
        assert!(!resynced.has_more);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_pages_context_ref_with_session_and_task_scope() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-context-page-ref");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();

        let page = facade
            .page_context_ref(&handle.session_id, "ref://event/1")
            .unwrap();

        assert_eq!(page.session_id, handle.session_id);
        assert_eq!(page.task_id, handle.task_id);
        assert_eq!(page.reference, "ref://event/1");
        assert_eq!(page.sequence, 1);
        assert_eq!(page.event_type, "session.created");
        assert_eq!(page.event_id, "evt_0001");
        assert!(page.projected_message.contains("session.created#1"));
        assert!(!page.payload_json.trim().is_empty());
        assert!(facade
            .page_context_ref(&handle.session_id, "event/1")
            .unwrap_err()
            .contains("unsupported context reference"));
        assert!(facade
            .page_context_ref(&handle.session_id, "ref://event/999")
            .unwrap_err()
            .contains("unknown context reference"));
        assert!(facade
            .page_context_ref("missing-session", "ref://event/1")
            .unwrap_err()
            .contains("unknown session_id"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_rejects_context_ref_page_back_for_mismatched_session_or_task_event() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-context-page-ref-scope");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let foreign_event = r#"{"event_id":"foreign_evt","schema_version":"v0","project_id":"proj","session_id":"foreign_session","task_id":"foreign_task","sequence":1,"event_type":"tool.result_recorded","actor":"runtime","created_at":"now","payload":{"tool_call_id":"tc_foreign","preview":"foreign raw event must not page back"},"prev_hash":null,"hash":"foreign_hash"}"#;

        facade
            .ingest_agent_event_jsonl_line(&handle.session_id, foreign_event)
            .unwrap();
        let next_ref = format!(
            "ref://event/{}",
            facade
                .get_session_snapshot(&handle.session_id)
                .unwrap()
                .event_count
        );
        let error = facade
            .page_context_ref(&handle.session_id, &next_ref)
            .unwrap_err();

        assert!(error.contains("belongs to session"));
        assert!(error.contains("foreign_session"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_previews_read_only_tools() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-preview");
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "ResearchCode facade\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let result = facade
            .preview_tool(
                &root,
                "facade_file_read_1",
                "file.read",
                ToolExecutionArgs {
                    path: Some("README.md".to_string()),
                    max_bytes: Some(128),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(result.ok);
        assert!(result.preview.contains("README.md"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_reopens_completed_session_for_next_tui_turn() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-next-turn");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "ResearchCode next turn\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        {
            let mut sessions = facade.sessions.lock().unwrap();
            let record = sessions.get_mut(&handle.session_id).unwrap();
            record.session.start_review().unwrap();
            record.session.complete_after_review().unwrap();
            assert_eq!(record.session.state(), AgentState::Completed);
        }

        facade
            .submit_user_message(&handle.session_id, "下一轮继续")
            .unwrap();
        let read = facade
            .execute_session_tool(
                &handle.session_id,
                "next_turn_read",
                "file.read",
                ToolExecutionArgs {
                    path: Some("README.md".to_string()),
                    max_bytes: Some(256),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(read, FacadeToolOutcome::Executed(result) if result.ok));
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream
            .jsonl
            .contains("\"event_type\":\"session.turn_started\""));
        assert!(stream.jsonl.contains("\"from_state\":\"Completed\""));
        assert!(stream.jsonl.contains("next_turn_read"));
        let snapshot = facade.get_session_snapshot(&handle.session_id).unwrap();
        assert_eq!(snapshot.state, AgentState::Executing);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_fast_auto_runs_safe_command_but_blocks_hard_deny() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-fast-auto");
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let safe = facade
            .execute_session_tool(
                &handle.session_id,
                "safe_find",
                "shell.command",
                ToolExecutionArgs {
                    command: Some("find . -maxdepth 0".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(safe, FacadeToolOutcome::Executed(_)));
        let denied = facade
            .execute_session_tool(
                &handle.session_id,
                "deny_rm",
                "shell.command",
                ToolExecutionArgs {
                    command: Some("rm -rf .".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(denied, FacadeToolOutcome::BlockedByPolicy(_)));
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert_eq!(
            stream
                .jsonl
                .matches("\"event_type\":\"permission.decision.recorded\"")
                .count(),
            1
        );
        assert!(stream.jsonl.contains("\"tool_id\":\"shell.command\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_resumes_session_from_eventlog() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-resume");
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::Qwen,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let event_path = root.join("events.jsonl");
        facade
            .export_events(&handle.session_id, &event_path)
            .unwrap();
        let resumed = facade.resume_session_from_eventlog(&event_path).unwrap();
        let snapshot = facade.get_session_snapshot(&resumed.session_id).unwrap();
        assert!(snapshot.event_count >= 4);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_persists_project_permission_rule() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-project-policy");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let first = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        let args = ToolExecutionArgs {
            command: Some("find . -maxdepth 0".to_string()),
            ..ToolExecutionArgs::default()
        };
        let pending = facade
            .execute_session_tool(
                &first.session_id,
                "policy_find_1",
                "shell.command",
                args.clone(),
            )
            .unwrap();
        assert!(matches!(
            pending,
            FacadeToolOutcome::RequiresPermission { .. }
        ));
        let continued = facade
            .continue_session_tool_after_permission(
                &first.session_id,
                "policy_find_1",
                "shell.command",
                args.clone(),
                PermissionDecisionKind::AllowProjectRule,
            )
            .unwrap();
        assert!(matches!(continued, FacadeToolOutcome::Executed(_)));
        assert!(root.join("artifacts/permission_policy.tsv").exists());

        let second = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        let auto = facade
            .execute_session_tool(&second.session_id, "policy_find_2", "shell.command", args)
            .unwrap();
        assert!(matches!(auto, FacadeToolOutcome::Executed(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_keeps_allow_session_rule_in_session() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-session-policy");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        let args = ToolExecutionArgs {
            command: Some("find . -maxdepth 0".to_string()),
            ..ToolExecutionArgs::default()
        };
        let pending = facade
            .execute_session_tool(
                &handle.session_id,
                "session_find_1",
                "shell.command",
                args.clone(),
            )
            .unwrap();
        assert!(matches!(
            pending,
            FacadeToolOutcome::RequiresPermission { .. }
        ));
        facade
            .continue_session_tool_after_permission(
                &handle.session_id,
                "session_find_1",
                "shell.command",
                args.clone(),
                PermissionDecisionKind::AllowSession,
            )
            .unwrap();
        let auto = facade
            .execute_session_tool(&handle.session_id, "session_find_2", "shell.command", args)
            .unwrap();
        assert!(matches!(auto, FacadeToolOutcome::Executed(_)));
        assert!(!root.join("artifacts/permission_policy.tsv").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_fast_auto_sensitive_write_requires_permission() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-sensitive-policy");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();

        let args = ToolExecutionArgs {
            path: Some(".env".to_string()),
            content: Some("TOKEN=redacted\n".to_string()),
            ..ToolExecutionArgs::default()
        };
        let outcome = facade
            .execute_session_tool(
                &handle.session_id,
                "sensitive_write",
                "file.write",
                args.clone(),
            )
            .unwrap();

        assert!(matches!(
            outcome,
            FacadeToolOutcome::RequiresPermission {
                request_type: PermissionRequestType::FileWrite,
                ..
            }
        ));
        assert!(!root.join(".env").exists());
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert_eq!(
            stream
                .jsonl
                .matches("\"event_type\":\"permission.decision.recorded\"")
                .count(),
            1
        );
        assert!(stream.jsonl.contains("\"tool_id\":\"file.write\""));
        assert!(stream.jsonl.contains("\"permission.requested\""));

        let continued = facade
            .continue_session_tool_after_permission(
                &handle.session_id,
                "sensitive_write",
                "file.write",
                args,
                PermissionDecisionKind::AllowOnce,
            )
            .unwrap();
        assert!(matches!(
            continued,
            FacadeToolOutcome::Executed(ToolExecutionResult { ok: true, .. })
        ));
        assert_eq!(
            fs::read_to_string(root.join(".env")).unwrap(),
            "TOKEN=redacted\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_sensitive_patch_defers_validation_until_permission() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-sensitive-patch");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join(".env"), "TOKEN=old\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let args = ToolExecutionArgs {
            path: Some(".env".to_string()),
            old_string: Some("TOKEN=old".to_string()),
            new_string: Some("TOKEN=new".to_string()),
            base_hash: Some(stable_text_hash("TOKEN=old\n")),
            ..ToolExecutionArgs::default()
        };

        let outcome = facade
            .execute_session_tool(
                &handle.session_id,
                "sensitive_patch",
                "patch.apply",
                args.clone(),
            )
            .unwrap();

        assert!(matches!(
            outcome,
            FacadeToolOutcome::RequiresPermission {
                request_type: PermissionRequestType::FileWrite,
                ..
            }
        ));
        assert_eq!(
            fs::read_to_string(root.join(".env")).unwrap(),
            "TOKEN=old\n"
        );
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("\"permission.requested\""));
        assert!(!stream.jsonl.contains("\"patch.proposal_created\""));

        let continued = facade
            .continue_session_tool_after_permission(
                &handle.session_id,
                "sensitive_patch",
                "patch.apply",
                args,
                PermissionDecisionKind::AllowOnce,
            )
            .unwrap();
        assert!(matches!(
            continued,
            FacadeToolOutcome::Executed(ToolExecutionResult { ok: true, .. })
        ));
        assert_eq!(
            fs::read_to_string(root.join(".env")).unwrap(),
            "TOKEN=new\n"
        );
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("\"patch.proposal_created\""));
        let _ = fs::remove_dir_all(root);
    }

    fn install_pending_native_shell_decision(
        facade: &RuntimeFacade,
        handle: &RuntimeSessionHandle,
        tool_call_id: &str,
        permission_id: &str,
        command: &str,
        turn_id: Option<&str>,
    ) {
        let pending_tool = PendingNativeToolExecution {
            step_index: 0,
            tool_call_id: tool_call_id.to_string(),
            tool_id: "shell.command".to_string(),
            permission_id: permission_id.to_string(),
            request_type: PermissionRequestType::Command,
            patch_id: None,
            args: ToolExecutionArgs {
                command: Some(command.to_string()),
                ..ToolExecutionArgs::default()
            },
        };
        let mut sessions = facade.sessions.lock().unwrap();
        let record = sessions.get_mut(&handle.session_id).unwrap();
        if let Some(turn_id) = turn_id {
            record
                .session
                .begin_interactive_turn(turn_id, "test_pending_native_shell")
                .unwrap();
        }
        record
            .session
            .record_tool_call_requested(&pending_tool.tool_call_id, &pending_tool.tool_id)
            .and_then(|_| {
                record.session.request_permission(
                    pending_tool.permission_id.clone(),
                    PermissionRequestType::Command,
                    Some(pending_tool.tool_id.clone()),
                )
            })
            .unwrap();
        record.pending_native_decision = Some(RuntimePendingNativeDecision {
            session_id: handle.session_id.clone(),
            turn_id: record.session.current_turn_id().map(str::to_string),
            permission_id: pending_tool.permission_id.clone(),
            provider_tool_call_id: None,
            tool_call_id: pending_tool.tool_call_id.clone(),
            tool_id: pending_tool.tool_id.clone(),
            args: pending_tool.args.clone(),
            blocked_event_jsonl: record.session.export_events_jsonl(),
            resume_strategy: "test_native_loop".to_string(),
            created_timestamp: "test".to_string(),
            pending_tool,
        });
    }

    #[test]
    fn facade_permission_decision_executes_pending_native_tool_with_outcome() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-native-permission");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        let pending_tool = PendingNativeToolExecution {
            step_index: 0,
            tool_call_id: "native_loop_tool_0".to_string(),
            tool_id: "shell.command".to_string(),
            permission_id: "native_loop_command_perm_0".to_string(),
            request_type: PermissionRequestType::Command,
            patch_id: None,
            args: ToolExecutionArgs {
                command: Some("find . -maxdepth 0".to_string()),
                ..ToolExecutionArgs::default()
            },
        };
        {
            let mut sessions = facade.sessions.lock().unwrap();
            let record = sessions.get_mut(&handle.session_id).unwrap();
            record
                .session
                .record_tool_call_requested_with_provider_id(
                    &pending_tool.tool_call_id,
                    Some("toolu_shell_1".to_string()),
                    &pending_tool.tool_id,
                )
                .and_then(|_| {
                    record.session.request_permission(
                        pending_tool.permission_id.clone(),
                        PermissionRequestType::Command,
                        Some(pending_tool.tool_id.clone()),
                    )
                })
                .unwrap();
            record.pending_native_decision = Some(RuntimePendingNativeDecision {
                session_id: handle.session_id.clone(),
                turn_id: record.session.current_turn_id().map(str::to_string),
                permission_id: pending_tool.permission_id.clone(),
                provider_tool_call_id: Some("toolu_shell_1".to_string()),
                tool_call_id: pending_tool.tool_call_id.clone(),
                tool_id: pending_tool.tool_id.clone(),
                args: pending_tool.args.clone(),
                blocked_event_jsonl: record.session.export_events_jsonl(),
                resume_strategy: "test_native_loop".to_string(),
                created_timestamp: "test".to_string(),
                pending_tool,
            });
        }

        assert!(facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                "wrong_permission",
                PermissionDecisionKind::AllowOnce,
            )
            .is_err());
        {
            let sessions = facade.sessions.lock().unwrap();
            assert!(sessions
                .get(&handle.session_id)
                .unwrap()
                .pending_native_decision
                .is_some());
        }

        let outcome = facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                "native_loop_command_perm_0",
                PermissionDecisionKind::AllowOnce,
            )
            .unwrap();
        assert!(outcome.tool_executed);
        assert!(outcome.model_continuation_required);
        assert_eq!(
            outcome.provider_tool_call_id.as_deref(),
            Some("toolu_shell_1")
        );
        assert_eq!(outcome.tool_id.as_deref(), Some("shell.command"));
        assert!(outcome.tool_result.as_ref().unwrap().ok);
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("runtime.permission_resume.started"));
        assert!(stream
            .jsonl
            .contains("runtime.permission_resume.tool_executed"));
        assert!(!stream
            .jsonl
            .contains("runtime.permission_resume.model_continuation_skipped"));
        assert!(stream.jsonl.contains("runtime.permission_resume.completed"));
        assert!(stream
            .jsonl
            .contains("\"model_continuation_required\":true"));
        assert!(!stream.jsonl.contains("permission_resume_tool_completed"));
        assert!(stream
            .jsonl
            .contains("\"provider_tool_call_id\":\"toolu_shell_1\""));
        let sessions = facade.sessions.lock().unwrap();
        assert!(sessions
            .get(&handle.session_id)
            .unwrap()
            .pending_native_decision
            .is_none());
        assert_eq!(
            sessions.get(&handle.session_id).unwrap().session.state(),
            AgentState::Executing
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn failed_permission_resume_reports_no_model_continuation() {
        let root =
            std::env::temp_dir().join("researchcode-runtime-facade-native-permission-failed-tool");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        install_pending_native_shell_decision(
            &facade,
            &handle,
            "native_loop_tool_failed_resume",
            "native_loop_failed_resume_perm_0",
            "false",
            None,
        );

        let outcome = facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                "native_loop_failed_resume_perm_0",
                PermissionDecisionKind::AllowOnce,
            )
            .unwrap();
        assert!(outcome.tool_executed);
        assert!(!outcome.model_continuation_required);
        assert_eq!(
            outcome.error_code.as_deref(),
            Some("permission_resume_tool_failed")
        );
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("runtime.permission_resume.completed"));
        assert!(stream
            .jsonl
            .contains("\"model_continuation_required\":false"));
        let sessions = facade.sessions.lock().unwrap();
        assert_eq!(
            sessions.get(&handle.session_id).unwrap().session.state(),
            AgentState::DiagnosingFailure
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn double_native_permission_approval_does_not_execute_twice() {
        let root =
            std::env::temp_dir().join("researchcode-runtime-facade-native-permission-double");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        install_pending_native_shell_decision(
            &facade,
            &handle,
            "native_loop_tool_double_resume",
            "native_loop_double_resume_perm_0",
            "find . -maxdepth 0",
            None,
        );

        let first = facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                "native_loop_double_resume_perm_0",
                PermissionDecisionKind::AllowOnce,
            )
            .unwrap();
        assert!(first.tool_executed);
        assert!(first.tool_result.as_ref().unwrap().ok);

        assert!(facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                "native_loop_double_resume_perm_0",
                PermissionDecisionKind::AllowOnce,
            )
            .is_err());
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert_eq!(
            stream
                .jsonl
                .matches("runtime.permission_resume.tool_executed")
                .count(),
            1
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_cancel_clears_pending_native_permission() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-cancel-pending-native");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        let pending_tool = PendingNativeToolExecution {
            step_index: 0,
            tool_call_id: "native_loop_tool_cancel".to_string(),
            tool_id: "shell.command".to_string(),
            permission_id: "native_loop_cancel_perm_0".to_string(),
            request_type: PermissionRequestType::Command,
            patch_id: None,
            args: ToolExecutionArgs {
                command: Some("find . -maxdepth 0".to_string()),
                ..ToolExecutionArgs::default()
            },
        };
        {
            let mut sessions = facade.sessions.lock().unwrap();
            let record = sessions.get_mut(&handle.session_id).unwrap();
            record
                .session
                .record_tool_call_requested(&pending_tool.tool_call_id, &pending_tool.tool_id)
                .and_then(|_| {
                    record.session.request_permission(
                        pending_tool.permission_id.clone(),
                        PermissionRequestType::Command,
                        Some(pending_tool.tool_id.clone()),
                    )
                })
                .unwrap();
            record.pending_native_decision = Some(RuntimePendingNativeDecision {
                session_id: handle.session_id.clone(),
                turn_id: record.session.current_turn_id().map(str::to_string),
                permission_id: pending_tool.permission_id.clone(),
                provider_tool_call_id: None,
                tool_call_id: pending_tool.tool_call_id.clone(),
                tool_id: pending_tool.tool_id.clone(),
                args: pending_tool.args.clone(),
                blocked_event_jsonl: record.session.export_events_jsonl(),
                resume_strategy: "test_native_loop".to_string(),
                created_timestamp: "test".to_string(),
                pending_tool,
            });
        }

        facade.cancel_session(&handle.session_id).unwrap();
        let sessions = facade.sessions.lock().unwrap();
        let record = sessions.get(&handle.session_id).unwrap();
        assert!(record.pending_native_decision.is_none());
        assert!(record.session.pending_permission_id().is_none());
        assert_eq!(record.session.state(), AgentState::Cancelled);
        let jsonl = record.session.export_events_jsonl();
        assert!(jsonl.contains("runtime.turn_cancel_requested"));
        assert!(jsonl.contains("permission.cleared"));
        drop(sessions);
        assert!(facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                "native_loop_cancel_perm_0",
                PermissionDecisionKind::AllowOnce,
            )
            .is_err());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_native_permission_decision_is_rejected_without_executing_tool() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-stale-native-permission");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        let marker = root.join("stale_permission_executed_marker");
        let pending_tool = PendingNativeToolExecution {
            step_index: 0,
            tool_call_id: "native_loop_tool_stale".to_string(),
            tool_id: "shell.command".to_string(),
            permission_id: "native_loop_stale_perm_0".to_string(),
            request_type: PermissionRequestType::Command,
            patch_id: None,
            args: ToolExecutionArgs {
                command: Some("touch stale_permission_executed_marker".to_string()),
                ..ToolExecutionArgs::default()
            },
        };
        {
            let mut sessions = facade.sessions.lock().unwrap();
            let record = sessions.get_mut(&handle.session_id).unwrap();
            record
                .session
                .begin_interactive_turn("turn_a", "test_stale_permission")
                .unwrap();
            record
                .session
                .record_tool_call_requested(&pending_tool.tool_call_id, &pending_tool.tool_id)
                .and_then(|_| {
                    record.session.request_permission(
                        pending_tool.permission_id.clone(),
                        PermissionRequestType::Command,
                        Some(pending_tool.tool_id.clone()),
                    )
                })
                .unwrap();
            record.pending_native_decision = Some(RuntimePendingNativeDecision {
                session_id: handle.session_id.clone(),
                turn_id: Some("turn_a".to_string()),
                permission_id: pending_tool.permission_id.clone(),
                provider_tool_call_id: None,
                tool_call_id: pending_tool.tool_call_id.clone(),
                tool_id: pending_tool.tool_id.clone(),
                args: pending_tool.args.clone(),
                blocked_event_jsonl: record.session.export_events_jsonl(),
                resume_strategy: "test_native_loop".to_string(),
                created_timestamp: "test".to_string(),
                pending_tool,
            });
            record
                .session
                .begin_interactive_turn("turn_b", "newer_turn_before_approval")
                .unwrap();
        }

        let error = facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                "native_loop_stale_perm_0",
                PermissionDecisionKind::AllowOnce,
            )
            .unwrap_err();
        assert!(error.contains("stale permission decision"));
        assert!(!marker.exists());
        let sessions = facade.sessions.lock().unwrap();
        let record = sessions.get(&handle.session_id).unwrap();
        assert!(record.pending_native_decision.is_none());
        assert!(record.session.pending_permission_id().is_none());
        assert!(record
            .session
            .export_events_jsonl()
            .contains("permission.cleared"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn native_pending_permission_identity_uses_merged_tool_request_id() {
        let root = std::env::temp_dir()
            .join("researchcode-runtime-facade-native-permission-merged-identity");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();
        let pending_tool = PendingNativeToolExecution {
            step_index: 50,
            tool_call_id: "native_loop_v2_tool_50".to_string(),
            tool_id: "shell.command".to_string(),
            permission_id: "native_loop_v2_command_perm_50".to_string(),
            request_type: PermissionRequestType::Command,
            patch_id: None,
            args: ToolExecutionArgs {
                command: Some("swift test --help".to_string()),
                ..ToolExecutionArgs::default()
            },
        };
        {
            let mut sessions = facade.sessions.lock().unwrap();
            let record = sessions.get_mut(&handle.session_id).unwrap();
            record
                .session
                .record_tool_call_requested_with_provider_id(
                    "native_loop_v2_tool_50_loop_1204",
                    Some("toolu_v2_5_0".to_string()),
                    "shell.command",
                )
                .and_then(|_| {
                    record.session.request_permission(
                        pending_tool.permission_id.clone(),
                        PermissionRequestType::Command,
                        Some(pending_tool.tool_id.clone()),
                    )
                })
                .unwrap();
            let result = NativeAgentLoopResult {
                status: NativeAgentLoopStatus::Blocked,
                final_state: AgentState::WaitingForToolApproval,
                event_count: 0,
                tool_call_count: 1,
                model_call_count: 1,
                prompt_tokens: 0,
                completion_tokens: 0,
                reasoning_tokens: 0,
                prompt_cache_hit_tokens: 0,
                prompt_cache_miss_tokens: 0,
                event_jsonl: String::new(),
                pending_tool: Some(pending_tool),
            };
            record.pending_native_decision = runtime_pending_native_decision_from_result(
                &handle.session_id,
                &record.session,
                &result,
                "deepseek_native_loop",
            )
            .unwrap();
        }
        let sessions = facade.sessions.lock().unwrap();
        let pending = sessions
            .get(&handle.session_id)
            .unwrap()
            .pending_native_decision
            .as_ref()
            .unwrap();
        assert_eq!(pending.tool_call_id, "native_loop_v2_tool_50_loop_1204");
        assert_eq!(
            pending.provider_tool_call_id.as_deref(),
            Some("toolu_v2_5_0")
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_routes_plan_enter_to_plan_approval_not_permission() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-plan-approval");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let outcome = facade
            .execute_session_tool(
                &handle.session_id,
                "plan_enter_1",
                "plan.enter",
                ToolExecutionArgs {
                    content: Some("Plan: read, patch, test".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(
            outcome,
            FacadeToolOutcome::RequiresPlanApproval { .. }
        ));
        let snapshot = facade.get_session_snapshot(&handle.session_id).unwrap();
        assert_eq!(snapshot.pending_plan_approval_count, 1);
        assert_eq!(snapshot.pending_permission_count, 0);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn plan_approval_closes_plan_enter_tool_call_in_openai_history() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-plan-history");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let outcome = facade
            .execute_session_tool(
                &handle.session_id,
                "plan_enter_history",
                "plan.enter",
                ToolExecutionArgs {
                    content: Some("Plan: inspect, edit, verify".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        let plan_approval_id = match outcome {
            FacadeToolOutcome::RequiresPlanApproval { plan_approval_id } => plan_approval_id,
            other => panic!("expected plan approval, got {other:?}"),
        };
        facade
            .submit_plan_decision(
                &handle.session_id,
                &plan_approval_id,
                PlanApprovalDecisionKind::Approve,
            )
            .unwrap();

        let history = facade
            .conversation_history_openai_json(&handle.session_id)
            .unwrap();
        assert!(history.contains("\"tool_calls\""));
        assert!(history.contains("\"id\":\"plan_enter_history\""));
        assert!(history.contains("\"tool_call_id\":\"plan_enter_history\""));
        assert!(history.contains("PlanApproval: approve"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_routes_ask_user_to_waiting_for_user_not_permission() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-ask-user");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let outcome = facade
            .execute_session_tool(
                &handle.session_id,
                "ask_user_1",
                "ask_user",
                ToolExecutionArgs {
                    query: Some("Which file should I inspect first?".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(outcome, FacadeToolOutcome::Executed(_)));
        let snapshot = facade.get_session_snapshot(&handle.session_id).unwrap();
        assert_eq!(snapshot.state, AgentState::WaitingForUser);
        assert_eq!(snapshot.pending_permission_count, 0);
        assert_eq!(snapshot.pending_plan_approval_count, 0);
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("user.question_requested"));
        assert!(!stream.jsonl.contains("permission.requested"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runtime_deepseek_token_budget_is_not_chat_sized_for_agent_work() {
        assert_eq!(deepseek_runtime_max_tokens_for_prompt("你好"), 8_192);
        assert_eq!(
            deepseek_runtime_max_tokens_for_prompt("帮我写个html小程序"),
            16_384
        );
        assert_eq!(
            deepseek_runtime_max_tokens_for_prompt("深度解析这个代码库"),
            20_000
        );
    }

    #[test]
    fn facade_context_v2_replays_memory_and_file_state() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-context-v2");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "ResearchCode context v2\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::Qwen,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        facade
            .submit_user_message(&handle.session_id, "remember this task")
            .unwrap();
        let read = facade
            .execute_session_tool(
                &handle.session_id,
                "context_read",
                "file.read",
                ToolExecutionArgs {
                    path: Some("README.md".to_string()),
                    max_bytes: Some(512),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(read, FacadeToolOutcome::Executed(_)));
        let bundle = facade.build_context_bundle(&handle.session_id).unwrap();
        assert!(bundle
            .items
            .iter()
            .any(|item| item.source == "runtime.session_memory"));
        assert!(bundle
            .items
            .iter()
            .any(|item| item.source.contains("file_state:README.md")));
        let sessions = facade
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions.get(&handle.session_id).unwrap();
        let directive = build_runtime_evidence_directive(record);
        assert!(directive.contains("# Runtime Evidence Ledger"));
        assert!(directive.contains("Do not reread covered plan/file ranges"));
        assert!(directive.contains("README.md"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn live_zero_tool_call_budget_stays_uncapped() {
        assert_eq!(
            effective_live_max_tool_calls(0, "请继续按照plan进行编码"),
            0
        );
        assert_eq!(effective_live_max_tool_calls(0, "可以开始写了"), 0);
        assert_eq!(effective_live_max_tool_calls(0, "status?"), 0);
        assert_eq!(effective_live_max_tool_calls(3, "继续完成修复"), 64);
        assert_eq!(effective_live_max_tool_calls(3, "可以开始写了"), 64);
        assert_eq!(effective_live_max_tool_calls(99, "status?"), 99);
        assert_eq!(effective_live_max_tool_calls(999, "status?"), 256);
        assert_eq!(effective_live_max_iterations(0), 0);
        assert_eq!(effective_live_max_iterations(99), 99);
        assert_eq!(effective_live_max_iterations(999), 256);
    }

    #[test]
    fn session_memory_excludes_plateau_fallback_notes() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-plateau-memory-filter");
        let _ = fs::remove_dir_all(&root);
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let mut sessions = facade
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let record = sessions.get_mut(&handle.session_id).unwrap();
        remember_session(
            record,
            "assistant: runtime 已停止本轮工具循环，但没有新的可展示证据".to_string(),
        );
        remember_session(
            record,
            "tool file.read ok=true preview=README.md".to_string(),
        );
        assert_eq!(record.session_memory.len(), 1);
        let directive = build_runtime_evidence_directive(record);
        assert!(!directive.contains("runtime 已停止本轮工具循环"));
        assert!(directive.contains("README.md"));
        drop(sessions);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn trim_for_memory_compacts_unicode_without_panicking() {
        let value = "继续按照plan工作啊🙂\n".repeat(80);
        let trimmed = trim_for_memory(&value, 120);

        assert!(trimmed.contains("继续"));
        assert!(trimmed.contains("...truncated"));
        assert!(!trimmed.contains('\n'));
    }

    #[test]
    fn facade_context_bundle_excludes_conversation_summary_and_exposes_openai_json() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-conversation-context");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::Qwen,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        facade
            .submit_user_message(&handle.session_id, "carry this conversation forward")
            .unwrap();

        let bundle = facade.build_context_bundle(&handle.session_id).unwrap();
        assert!(!bundle
            .items
            .iter()
            .any(|item| item.source.starts_with("conversation_history")));

        let history_json = facade
            .conversation_history_openai_json(&handle.session_id)
            .unwrap();
        assert!(history_json.contains("\"role\":\"user\""));
        assert!(history_json.contains("carry this conversation forward"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_enforces_read_before_file_edit() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "researchcode-runtime-facade-read-before-write-{nonce}"
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("src.txt"), "alpha\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let blocked = facade
            .execute_session_tool(
                &handle.session_id,
                "edit_without_read",
                "file.edit",
                ToolExecutionArgs {
                    path: Some("src.txt".to_string()),
                    old_string: Some("alpha".to_string()),
                    new_string: Some("beta".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(blocked, FacadeToolOutcome::BlockedByPolicy(_)));
        let read = facade
            .execute_session_tool(
                &handle.session_id,
                "read_before_edit",
                "file.read",
                ToolExecutionArgs {
                    path: Some("src.txt".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(read, FacadeToolOutcome::Executed(_)));
        let edited = facade
            .execute_session_tool(
                &handle.session_id,
                "edit_after_read",
                "file.edit",
                ToolExecutionArgs {
                    path: Some("src.txt".to_string()),
                    old_string: Some("alpha".to_string()),
                    new_string: Some("beta".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(edited, FacadeToolOutcome::Executed(_)));
        assert_eq!(fs::read_to_string(root.join("src.txt")).unwrap(), "beta\n");
        let edited_again = facade
            .execute_session_tool(
                &handle.session_id,
                "edit_after_runtime_write",
                "file.edit",
                ToolExecutionArgs {
                    path: Some("src.txt".to_string()),
                    old_string: Some("beta".to_string()),
                    new_string: Some("gamma".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(edited_again, FacadeToolOutcome::Executed(_)));
        assert_eq!(fs::read_to_string(root.join("src.txt")).unwrap(), "gamma\n");
        let stale = facade
            .execute_session_tool(
                &handle.session_id,
                "edit_after_external_change",
                "file.edit",
                ToolExecutionArgs {
                    path: Some("src.txt".to_string()),
                    old_string: Some("gamma".to_string()),
                    new_string: Some("delta".to_string()),
                    base_hash: Some("stale_hash".to_string()),
                    ..ToolExecutionArgs::default()
                },
            )
            .unwrap();
        assert!(matches!(stale, FacadeToolOutcome::BlockedByPolicy(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_owns_deepseek_native_loop_session_events() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-native-loop");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "ResearchCode native facade loop\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"file.read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Done after file read."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let result = facade
            .run_deepseek_agent_loop_with_transport(
                &transport,
                &handle.session_id,
                "Read README and finish.",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(
            result.status,
            crate::native_agent_loop::NativeAgentLoopStatus::Completed
        );
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("model.call_started"));
        assert!(stream.jsonl.contains("tool.call_completed"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_injects_openai_conversation_history_tool_call_ids_on_next_turn() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-openai-history");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("README.md"),
            "ResearchCode conversation history\n",
        )
        .unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();

        let first_transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_history_read","name":"file.read","input":{}}}
data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"README.md\"}"}}
data: {"type":"content_block_stop","index":0}
data: [DONE]"#
                    .to_string(),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"First turn read README."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let first = facade
            .run_deepseek_agent_loop_with_transport(
                &first_transport,
                &handle.session_id,
                "Read README and finish.",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(first.final_state, AgentState::Completed);

        let history_json = facade
            .conversation_history_openai_json(&handle.session_id)
            .unwrap();
        assert!(history_json.contains("\"tool_calls\""));
        assert!(history_json.contains("\"tool_call_id\":\"toolu_history_read\""));

        let second_transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Second turn saw prior tool_call_id."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let second = facade
            .run_deepseek_agent_loop_with_transport(
                &second_transport,
                &handle.session_id,
                "Continue from the previous tool evidence.",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(second.final_state, AgentState::Completed);
        let second_requests = second_transport.sent_requests();
        assert!(!second_requests.is_empty());
        let initial_body = &second_requests[0].body_json;
        assert!(!initial_body.contains("Conversation History (OpenAI JSON)"));
        assert!(initial_body.contains("toolu_history_read"));
        let body: serde_json::Value = serde_json::from_str(initial_body).unwrap();
        let messages = body
            .get("messages")
            .and_then(|value| value.as_array())
            .unwrap();
        assert!(messages.len() > 1);
        assert!(messages.iter().any(|message| {
            message
                .get("content")
                .and_then(|value| value.as_str())
                .is_some_and(|content| content.contains("toolu_history_read"))
        }));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_shell_permission_resume_injects_tool_result_on_next_turn() {
        let root =
            std::env::temp_dir().join("researchcode-runtime-facade-shell-permission-history");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::ManualReview,
            )
            .unwrap();

        let first_transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_shell_pwd","function":{"name":"shell.command","arguments":"{\"command\":\"pwd\"}"}}]}}]}
data: {"choices":[{"finish_reason":"tool_calls","delta":{}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let first = facade
            .run_deepseek_agent_loop_with_transport(
                &first_transport,
                &handle.session_id,
                "Run pwd, then continue explaining the workspace.",
                NativeProviderEndpoint::deepseek_v4_flash_openai(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(first.status, NativeAgentLoopStatus::Blocked);
        assert_eq!(first.final_state, AgentState::WaitingForToolApproval);
        assert!(first.pending_tool.is_some());
        let pending_id = first.pending_tool.as_ref().unwrap().permission_id.clone();

        let outcome = facade
            .submit_permission_decision_with_outcome(
                &handle.session_id,
                &pending_id,
                PermissionDecisionKind::AllowOnce,
            )
            .unwrap();
        assert!(outcome.tool_executed);
        assert!(outcome.model_continuation_required);
        assert_eq!(outcome.tool_id.as_deref(), Some("shell.command"));
        assert!(outcome.tool_result.as_ref().unwrap().ok);

        let second_transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"pwd completed; I can now continue from the shell result."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let second = facade
            .run_deepseek_agent_loop_with_transport(
                &second_transport,
                &handle.session_id,
                "Continue from the approved shell result.",
                NativeProviderEndpoint::deepseek_v4_flash_openai(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(second.status, NativeAgentLoopStatus::Completed);
        assert_eq!(second.final_state, AgentState::Completed);
        assert_eq!(second.tool_call_count, 0);
        assert_eq!(second.model_call_count, 1);

        let second_requests = second_transport.sent_requests();
        assert!(!second_requests.is_empty());
        let body = &second_requests[0].body_json;
        let request_json: serde_json::Value = serde_json::from_str(body).unwrap();
        let messages = request_json
            .get("messages")
            .and_then(|value| value.as_array())
            .expect("request includes messages");
        let assistant_tool_call = messages
            .iter()
            .find(|message| {
                message.get("role").and_then(|value| value.as_str()) == Some("assistant")
                    && message.get("tool_calls").is_some()
            })
            .expect("approved shell call is replayed as assistant tool_call");
        let replayed_tool_call = assistant_tool_call
            .get("tool_calls")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .expect("assistant tool_call exists");
        let replayed_tool_call_id = replayed_tool_call
            .get("id")
            .and_then(|value| value.as_str())
            .expect("assistant tool_call has id");
        assert_eq!(
            replayed_tool_call
                .get("function")
                .and_then(|value| value.get("name"))
                .and_then(|value| value.as_str()),
            Some("shell_command")
        );
        let tool_result = messages
            .iter()
            .find(|message| {
                message.get("role").and_then(|value| value.as_str()) == Some("tool")
                    && message.get("tool_call_id").and_then(|value| value.as_str())
                        == Some(replayed_tool_call_id)
            })
            .expect("approved shell result is replayed as matching tool message");
        let tool_result_content = tool_result
            .get("content")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        assert!(tool_result_content.contains("shell.command"));
        assert!(tool_result_content.contains("pwd"));

        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream
            .jsonl
            .contains("runtime.permission_resume.tool_executed"));
        assert!(stream
            .jsonl
            .contains("\"model_continuation_required\":true"));
        assert!(!stream
            .jsonl
            .contains("tool.alias_shell_list_to_directory_tool"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_native_loop_retries_transient_http_failure() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-native-http-retry");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 429,
                body: "rate limited".to_string(),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Recovered after retry."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);

        let result = facade
            .run_deepseek_agent_loop_with_transport(
                &transport,
                &handle.session_id,
                "Say done after retry.",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                4,
                4,
            )
            .unwrap();

        assert_eq!(result.final_state, AgentState::Completed);
        assert_eq!(transport.sent_requests().len(), 2);
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream
            .jsonl
            .contains("\"event_type\":\"model.http_retry_scheduled\""));
        assert!(stream
            .jsonl
            .contains("\"event_type\":\"agent.recovery.completed\""));
        assert!(stream.jsonl.contains("\"status_code\":429"));
        assert!(stream.jsonl.contains("\"retries\":1"));
        assert!(stream.jsonl.contains("Recovered after retry."));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_native_loop_merge_keeps_model_call_ids_pairable() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-call-id-pairing");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"Pairable call id."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        facade
            .run_deepseek_agent_loop_with_transport(
                &transport,
                &handle.session_id,
                "Say a short answer.",
                NativeProviderEndpoint::deepseek_v4_flash_openai(),
                4,
                4,
            )
            .unwrap();
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        let mut started = Vec::new();
        let mut completed = Vec::new();
        for line in stream.jsonl.lines() {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            let event_type = value
                .get("event_type")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            let call_id = value
                .get("payload")
                .and_then(|payload| payload.get("call_id"))
                .and_then(|value| value.as_str());
            match (event_type, call_id) {
                ("model.call_started", Some(call_id)) => started.push(call_id.to_string()),
                ("model.call_completed", Some(call_id)) => completed.push(call_id.to_string()),
                _ => {}
            }
        }
        assert!(!started.is_empty());
        assert_eq!(started, completed);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_ingests_live_sink_events_before_loop_completion_without_duplicates() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-live-sink-ingest");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Live sink done."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let mut endpoint = NativeProviderEndpoint::deepseek_v4_flash_anthropic();
        endpoint.api_key_env = "PATH".to_string();
        let mut observed_live_writeback = false;
        let mut forwarded_lifecycle_events = 0usize;
        let mut sink = |line: &str| {
            if line.contains("\"event_type\":\"session.state_changed\"")
                || line.contains("\"event_type\":\"session.created\"")
                || line.contains("\"event_type\":\"session.turn_started\"")
            {
                forwarded_lifecycle_events += 1;
            }
            if line.contains("\"event_type\":\"model.call_started\"") {
                let stream = facade.stream_agent_events(&handle.session_id).unwrap();
                observed_live_writeback = stream
                    .jsonl
                    .contains("\"event_type\":\"model.call_started\"");
            }
        };

        let result = facade
            .run_deepseek_agent_loop_with_transport_and_event_sink(
                &transport,
                &handle.session_id,
                "Say done.",
                endpoint,
                4,
                4,
                &mut sink,
            )
            .unwrap();

        assert_eq!(result.final_state, AgentState::Completed);
        assert!(observed_live_writeback);
        assert_eq!(forwarded_lifecycle_events, 0);
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert_eq!(
            stream
                .jsonl
                .matches("\"event_type\":\"model.call_started\"")
                .count(),
            1
        );
        assert!(stream.jsonl.contains("Live sink done."));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_direct_write_prompt_accepts_create_file_alias() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-direct-create-file");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let html = "<!doctype html><html><body><time>ok</time></body></html>";
        let escaped_html = html.replace('\\', "\\\\").replace('"', "\\\"");
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: format!(
                    "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"id\":\"call_create\",\"type\":\"function\",\"function\":{{\"name\":\"create_file\",\"arguments\":\"{{\\\"path\\\":\\\"clock.html\\\",\\\"content\\\":\\\"{escaped_html}\\\"}}\"}}}}]}}}}]}}\n\
data: [DONE]"
                ),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"已写入 clock.html。"}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let result = facade
            .run_deepseek_agent_loop_with_transport(
                &transport,
                &handle.session_id,
                "你直接写进文件夹啊",
                NativeProviderEndpoint::deepseek_v4_flash_openai(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(
            result.status,
            crate::native_agent_loop::NativeAgentLoopStatus::Blocked
        );
        assert!(!root.join("clock.html").exists());
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("\"requested_tool\":\"create_file\""));
        assert!(stream.jsonl.contains("\"resolved_tool\":\"file.write\""));
        assert!(stream
            .jsonl
            .contains("\"event_type\":\"permission.requested\""));
        assert!(!stream.jsonl.contains("UNKNOWN_TOOL"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_deepseek_loop_reopens_completed_session_for_next_turn() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-native-loop-reopen");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("README.md"), "ResearchCode loop reopen\n").unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();

        let first_transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"First turn done."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let first = facade
            .run_deepseek_agent_loop_with_transport(
                &first_transport,
                &handle.session_id,
                "第一轮",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(first.final_state, AgentState::Completed);

        let second_transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Second turn done."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let second = facade
            .run_deepseek_agent_loop_with_transport(
                &second_transport,
                &handle.session_id,
                "第二轮",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(second.final_state, AgentState::Completed);
        assert!(second.event_jsonl.contains("Second turn done."));
        assert!(second.model_call_count >= 1);

        fs::write(root.join("AGENTS.md"), "COMPRESS_ME ".repeat(75_000)).unwrap();
        let compacted_bundle = facade.build_context_bundle(&handle.session_id).unwrap();
        assert!(
            compacted_bundle
                .items
                .iter()
                .any(|item| item.source == "context.compaction"),
            "third turn should cross the facade context compaction boundary"
        );
        assert!(
            compacted_bundle.token_estimate()
                < crate::context_budget::DEEPSEEK_COMPACTION_THRESHOLD_TOKENS / 2
        );

        let third_transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Third turn done."}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let third = facade
            .run_deepseek_agent_loop_with_transport(
                &third_transport,
                &handle.session_id,
                "第三轮",
                NativeProviderEndpoint::deepseek_v4_flash_anthropic(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(third.final_state, AgentState::Completed);
        assert!(third.event_jsonl.contains("Third turn done."));
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("First turn done."));
        assert!(stream.jsonl.contains("Second turn done."));
        assert!(stream.jsonl.contains("Third turn done."));
        let snapshot = facade.get_session_snapshot(&handle.session_id).unwrap();
        assert_eq!(snapshot.state, AgentState::Completed);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_generation_prompt_exposes_fastauto_write_tools() {
        let root = std::env::temp_dir().join("researchcode-runtime-facade-fastauto-write-loop");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::DeepSeek,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let html = "<!doctype html><html><body><h1>RuntimeFacade Write</h1></body></html>";
        let escaped_html = html.replace('\\', "\\\\").replace('"', "\\\"");
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: format!(
                    "data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"id\":\"call_write\",\"type\":\"function\",\"function\":{{\"name\":\"file_write\",\"arguments\":\"{{\\\"path\\\":\\\"demo.html\\\",\\\"content\\\":\\\"{escaped_html}\\\"}}\"}}}}]}}}}]}}\n\
data: [DONE]"
                ),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"choices":[{"delta":{"content":"Created demo.html."}}]}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let result = facade
            .run_deepseek_agent_loop_with_transport(
                &transport,
                &handle.session_id,
                "帮我写个html小程序",
                NativeProviderEndpoint::deepseek_v4_flash_openai(),
                4,
                4,
            )
            .unwrap();
        assert_eq!(
            result.status,
            crate::native_agent_loop::NativeAgentLoopStatus::Blocked
        );
        assert!(!root.join("demo.html").exists());
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("\"tool_id\":\"file.write\""));
        assert!(stream
            .jsonl
            .contains("\"event_type\":\"permission.requested\""));
        assert!(stream.jsonl.contains("\"request_type\":\"file_write\""));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn facade_qwen_generation_prompt_uses_native_loop_and_write_tools() {
        let root =
            std::env::temp_dir().join("researchcode-runtime-facade-qwen-fastauto-write-loop");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let facade = RuntimeFacade::new(&root, root.join("artifacts"));
        let handle = facade
            .start_session(
                Some(root.clone()),
                RuntimeModelMode::Qwen,
                AutonomyMode::FastAuto,
            )
            .unwrap();
        let html = "<!doctype html><html><body><h1>Qwen Facade Write</h1></body></html>";
        let escaped_html = html.replace('\\', "\\\\").replace('"', "\\\"");
        let transport = crate::live_http_transport::ScriptedLiveHttpTransport::new(vec![
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: format!(
                    "data: {{\"model\":\"Qwen/Qwen3.6-27B\",\"choices\":[{{\"delta\":{{\"tool_calls\":[{{\"function\":{{\"name\":\"file_write\",\"arguments\":\"{{\\\"path\\\":\\\"qwen-demo.html\\\",\\\"content\\\":\\\"{escaped_html}\\\"}}\"}}}}]}}}}]}}\n\
data: {{\"usage\":{{\"prompt_tokens\":80,\"completion_tokens\":20,\"total_tokens\":100}}}}\n\
data: [DONE]"
                ),
            },
            crate::live_http_transport::LiveHttpResponse {
                status_code: 200,
                body: r#"data: {"model":"Qwen/Qwen3.6-27B","choices":[{"delta":{"content":"Created qwen-demo.html."}}]}
data: {"usage":{"prompt_tokens":90,"completion_tokens":16,"total_tokens":106}}
data: [DONE]"#
                    .to_string(),
            },
        ]);
        let mut endpoint = NativeProviderEndpoint::qwen36_27b_custom_endpoint();
        endpoint.base_url = "http://127.0.0.1:8000/v1/chat/completions".to_string();
        endpoint.live_calls_enabled_by_default = true;
        endpoint.api_key_env = "PATH".to_string();
        let result = facade
            .run_qwen_agent_loop_with_transport(
                &transport,
                &handle.session_id,
                "帮我写个html小程序",
                endpoint,
                4,
                4,
            )
            .unwrap();
        assert_eq!(
            result.status,
            crate::native_agent_loop::NativeAgentLoopStatus::Blocked
        );
        assert!(!root.join("qwen-demo.html").exists());
        let stream = facade.stream_agent_events(&handle.session_id).unwrap();
        assert!(stream.jsonl.contains("\"provider\":\"qwen\""));
        assert!(stream.jsonl.contains("\"tool_id\":\"file.write\""));
        assert!(stream
            .jsonl
            .contains("\"event_type\":\"permission.requested\""));
        assert!(stream.jsonl.contains("\"request_type\":\"file_write\""));
        let _ = fs::remove_dir_all(root);
    }
}
