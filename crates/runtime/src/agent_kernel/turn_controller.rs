//! Agent-kernel turn controller extraction target.
//!
//! `NativeTurnController` below remains the native ledger/context guard helper
//! from the pre-extraction loop. `NativeLoopTurnController` is the Phase 1
//! controller surface: it owns iteration gates and progress/convergence
//! decisions that used to live inline in `native_agent_loop.rs`.

use crate::agent_kernel::turn_state::TurnBudget;
use crate::agent_kernel::{
    ConvergenceEnforcer, ConvergenceVerdict, EvidenceLedger, ToolProgressDecision,
    TurnConvergenceVerdict, TurnState,
};
use crate::patch::stable_text_hash;
use crate::session::AgentSession;
use crate::tcml::{normalize_tool_id, parse_tool_arguments, ParsedToolCall};
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::Actor;
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};

pub use crate::native_turn_controller::{
    estimate_tokens, NativeContextGuardAction, NativeContextGuardReport, NativeTurnController,
};

pub trait TurnController<Ctx> {
    fn run_iteration(&mut self, ctx: &mut Ctx) -> IterationOutcome;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IterationOutcome {
    Continue { ids: NativeLoopIterationIds },
    Stop { reason: LoopStopReason },
    Block { reason: String },
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopStopReason {
    ProgressPlateau(&'static str),
    ConvergencePlateau(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLoopIterationIds {
    pub call_id: String,
    pub stream_id: String,
    pub transcript_id: String,
}

impl NativeLoopIterationIds {
    pub fn for_iteration(iteration: usize) -> Self {
        Self {
            call_id: format!("native_loop_v2_call_{iteration}"),
            stream_id: format!("native_loop_v2_stream_{iteration}"),
            transcript_id: format!("native_loop_v2_transcript_{iteration}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IterationPreflight {
    Continue(NativeLoopIterationIds),
    Interrupted,
    ToolLimitFailed,
    ToolLimitStop { reason: &'static str },
}

pub struct NativeLoopIterationContext<'a> {
    pub session: &'a mut AgentSession,
    pub turn_state: &'a mut TurnState,
    pub iteration: usize,
    pub tool_call_count: usize,
    pub effective_max_tool_calls: usize,
    pub has_last_tool_batch: bool,
    pub interrupt: &'a AtomicBool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolProgressReport {
    pub new_evidence_results: u32,
    pub duplicate_results: u32,
    pub error_results: u32,
    pub decision: ToolProgressDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopConvergenceAction {
    Continue,
    SoftWarning { reason: &'static str },
    Stop { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostToolBatchAction {
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolIterationControlAction {
    Continue,
    SoftWarning,
    Stop { reason: &'static str },
}

pub struct ToolIterationControlInput<'a> {
    pub iteration: usize,
    pub ok_results: usize,
    pub error_results: usize,
    pub duplicate_results: u32,
    pub distinct_keys_before: usize,
    pub progress_error_results: u32,
    pub repeated_error_signatures: Vec<(String, String)>,
    pub batch_signature: &'a str,
    pub seen_tool_batches: &'a [String],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolBatchGuardAction {
    Continue,
    UseSyntheticRecovery,
    Stop { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolBatchSignatureStatus {
    pub repeated_tool_batch: bool,
    pub alternating_batch: bool,
}

impl ToolBatchSignatureStatus {
    pub fn repeated_or_alternating(self) -> bool {
        self.repeated_tool_batch || self.alternating_batch
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContinuationStrategy {
    PlainEvidence,
    ProviderToolResult,
}

impl ContinuationStrategy {
    pub fn from_plain_evidence_preference(use_plain_evidence: bool) -> Self {
        if use_plain_evidence {
            Self::PlainEvidence
        } else {
            Self::ProviderToolResult
        }
    }

    pub fn event_label(self) -> &'static str {
        match self {
            Self::PlainEvidence => "plain_evidence_continuation",
            Self::ProviderToolResult => "provider_tool_result_continuation",
        }
    }
}

#[derive(Debug, Clone)]
pub struct NativeLoopTurnController {
    max_loop_guard_recoveries: usize,
    loop_guard_recovery_count: usize,
    max_convergence_soft_warnings: u32,
    convergence_soft_warning_reason: Option<&'static str>,
    convergence_soft_warning_count: u32,
    non_progress_recovery_count: usize,
    empty_visible_recovery_count: usize,
    dsml_leak_count: usize,
}

impl Default for NativeLoopTurnController {
    fn default() -> Self {
        Self {
            max_loop_guard_recoveries: 2,
            loop_guard_recovery_count: 0,
            max_convergence_soft_warnings: 3,
            convergence_soft_warning_reason: None,
            convergence_soft_warning_count: 0,
            non_progress_recovery_count: 0,
            empty_visible_recovery_count: 0,
            dsml_leak_count: 0,
        }
    }
}

impl NativeLoopTurnController {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn loop_guard_recovery_count(&self) -> usize {
        self.loop_guard_recovery_count
    }

    pub fn max_loop_guard_recoveries(&self) -> usize {
        self.max_loop_guard_recoveries
    }

    pub fn non_progress_recovery_count(&self) -> usize {
        self.non_progress_recovery_count
    }

    pub fn convergence_soft_warning_count(&self) -> u32 {
        self.convergence_soft_warning_count
    }

    pub fn empty_visible_recovery_count(&self) -> usize {
        self.empty_visible_recovery_count
    }

    pub fn dsml_leak_count(&self) -> usize {
        self.dsml_leak_count
    }

    pub fn begin_iteration(
        &self,
        session: &mut AgentSession,
        turn_state: &mut TurnState,
        iteration: usize,
        tool_call_count: usize,
        effective_max_tool_calls: usize,
        has_last_tool_batch: bool,
        interrupt: &AtomicBool,
    ) -> Result<IterationPreflight, String> {
        if interrupt.load(Ordering::Relaxed) {
            return Ok(IterationPreflight::Interrupted);
        }

        turn_state.iterations = iteration as u32 + 1;
        turn_state.tool_calls_used = tool_call_count as u32;

        if tool_call_count < effective_max_tool_calls {
            return Ok(IterationPreflight::Continue(
                NativeLoopIterationIds::for_iteration(iteration),
            ));
        }

        if !has_last_tool_batch {
            session
                .diagnose_failure()
                .map_err(|error| format!("{error:?}"))?;
            return Ok(IterationPreflight::ToolLimitFailed);
        }

        session
            .record_runtime_event(
                "agent.loop_budget_reached",
                Actor::Runtime,
                format!(
                    "{{\"reason\":\"max_tool_calls\",\"max_tool_calls\":{},\"action\":\"stop_with_structured_failure\"}}",
                    effective_max_tool_calls
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        Ok(IterationPreflight::ToolLimitStop {
            reason: "max_tool_calls",
        })
    }

    pub fn record_duplicate_suppression_summary(
        &self,
        session: &mut AgentSession,
        iteration: usize,
        duplicate_suppressed_count: u32,
    ) -> Result<(), String> {
        if duplicate_suppressed_count == 0 {
            return Ok(());
        }
        session
            .record_runtime_event(
                "agent.duplicate_suppression_summary",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"suppressed_count\":{},\"model_visible\":true,\"budget_counted\":false,\"visibility\":\"tool_result_hint\"}}",
                    iteration, duplicate_suppressed_count
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_max_iterations_budget_reached(
        &self,
        session: &mut AgentSession,
        max_iterations: u32,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.loop_budget_reached",
                Actor::Runtime,
                format!(
                    "{{\"reason\":\"max_iterations\",\"max_iterations\":{},\"action\":\"stop_with_structured_failure\"}}",
                    max_iterations
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn classify_tool_batch_signature(
        &self,
        seen_tool_batches: &[String],
        batch_signature: &str,
    ) -> ToolBatchSignatureStatus {
        let repeated_tool_batch = seen_tool_batches
            .iter()
            .any(|signature| signature == batch_signature);
        let alternating_batch = if !repeated_tool_batch && seen_tool_batches.len() >= 2 {
            seen_tool_batches
                .iter()
                .rev()
                .take(6)
                .any(|signature| signature == batch_signature)
        } else {
            false
        };
        ToolBatchSignatureStatus {
            repeated_tool_batch,
            alternating_batch,
        }
    }

    pub fn remember_tool_batch_if_novel(
        &self,
        turn_state: &mut TurnState,
        batch_signature: String,
        repeated_cached_observation_batch: bool,
    ) {
        if !repeated_cached_observation_batch
            && !turn_state
                .seen_tool_batches
                .iter()
                .any(|signature| signature == &batch_signature)
        {
            turn_state.seen_tool_batches.push(batch_signature);
        }
    }

    pub fn observe_tool_batch_guard(
        &mut self,
        session: &mut AgentSession,
        turn_state: &mut TurnState,
        iteration: usize,
        batch_signature: String,
        batch_status: ToolBatchSignatureStatus,
        repeated_cached_observation_batch: bool,
    ) -> Result<ToolBatchGuardAction, String> {
        if batch_status.repeated_or_alternating() && !repeated_cached_observation_batch {
            let exhausted = self.record_repeated_tool_batch(
                session,
                iteration,
                batch_status.alternating_batch,
            )?;
            if exhausted {
                self.record_turn_convergence_decision(
                    session,
                    turn_state,
                    iteration,
                    "NativeLoopTurnController.tool_batch_guard",
                    TurnConvergenceVerdict::Stop,
                    "repeated_tool_batch_exhausted_recovery",
                )?;
                self.record_repeated_tool_batch_stopped(session, iteration)?;
                return Ok(ToolBatchGuardAction::Stop {
                    reason: "repeated_tool_batch_exhausted_recovery",
                });
            }
            self.record_turn_convergence_decision(
                session,
                turn_state,
                iteration,
                "NativeLoopTurnController.tool_batch_guard",
                TurnConvergenceVerdict::SoftWarning,
                if batch_status.alternating_batch {
                    "alternating_tool_batch"
                } else {
                    "repeated_tool_batch"
                },
            )?;
            return Ok(ToolBatchGuardAction::UseSyntheticRecovery);
        }

        if repeated_cached_observation_batch {
            self.record_repeated_cached_observation_batch(session, iteration)?;
        } else {
            self.remember_tool_batch_if_novel(
                turn_state,
                batch_signature,
                repeated_cached_observation_batch,
            );
        }
        self.record_turn_convergence_decision(
            session,
            turn_state,
            iteration,
            "NativeLoopTurnController.tool_batch_guard",
            TurnConvergenceVerdict::Continue,
            "batch_guard_clear",
        )?;
        Ok(ToolBatchGuardAction::Continue)
    }

    pub fn record_repeated_cached_observation_batch(
        &self,
        session: &mut AgentSession,
        iteration: usize,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.loop_recovery",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":\"repeated_tool_batch\",\"status\":\"duplicate_observation_suppression\"}}",
                    iteration
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_streaming_batch_ready(
        &self,
        session: &mut AgentSession,
        iteration: usize,
        streamed_count: usize,
        parsed_count: usize,
        mismatch_error_count: usize,
        had_stream_parse_mismatch: bool,
    ) -> Result<(), String> {
        let action = if had_stream_parse_mismatch {
            "continue_with_streamed_results_size_mismatch"
        } else {
            "continue_with_streamed_tool_results"
        };
        session
            .record_runtime_event(
                "agent.tool.streaming_batch_ready",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"streamed_count\":{},\"parsed_count\":{},\"mismatch_error_count\":{},\"action\":\"{action}\"}}",
                    iteration, streamed_count, parsed_count, mismatch_error_count,
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn select_post_tool_batch_action(
        &self,
        _wants_tool_inventory: bool,
        _tool_inventory_ready: bool,
        _wants_file_generation: bool,
        _fast_auto_write_ready: bool,
    ) -> PostToolBatchAction {
        PostToolBatchAction::Continue
    }

    pub fn tool_batch_signature(&self, tool_calls: &[ParsedToolCall]) -> String {
        stable_text_hash(
            &tool_calls
                .iter()
                .map(|tool| {
                    let normalized_id = normalize_tool_id(&tool.tool_id);
                    let args = parse_tool_arguments(&tool.arguments_json);
                    crate::agent_kernel::observation_key(&normalized_id, &args).unwrap_or_else(
                        || {
                            // Keep non-observation tools deterministic without pretending they
                            // participate in ObservationCache coverage.
                            format!("{}:{}", normalized_id, tool.arguments_json.trim())
                        },
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub fn record_empty_visible_recovery(
        &mut self,
        session: &mut AgentSession,
        iteration: usize,
    ) -> Result<(), String> {
        self.empty_visible_recovery_count += 1;
        session
            .record_runtime_event(
                "agent.loop_recovery",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":\"empty_visible_response\",\"recovery_count\":{},\"action\":\"stop_with_structured_failure\"}}",
                    iteration, self.empty_visible_recovery_count
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_executable_dsml_fallback(
        &mut self,
        session: &mut AgentSession,
        iteration: usize,
        bytes: usize,
        family: &NativeModelFamily,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "tool_call.fallback_markup_parsed",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"markup\":\"dsml\",\"bytes\":{}}}",
                    iteration, bytes
                ),
            )
            .map_err(|error| format!("{error:?}"))?;

        if *family != NativeModelFamily::DeepSeek {
            return Ok(());
        }

        self.dsml_leak_count += 1;
        session
            .record_runtime_event(
                "deepseek.dsml.leak",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"markup\":\"dsml\",\"bytes\":{},\"source\":\"fallback_markup_parsed\",\"recovered\":true}}",
                    iteration, bytes
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        if self.dsml_leak_count == 3 {
            session
                .record_runtime_event(
                    "deepseek.dsml.leak_escalated",
                    Actor::Runtime,
                    format!(
                        "{{\"iteration\":{},\"leak_count\":{},\"severity\":\"warning\",\"next_action_hint\":\"Use the native provider tool-call channel only; do not emit DSML/tool markup in visible assistant content.\"}}",
                        iteration, self.dsml_leak_count
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
        }
        Ok(())
    }

    pub fn record_visible_tool_call_batch_assembled(
        &self,
        session: &mut AgentSession,
        family: &NativeModelFamily,
        iteration: usize,
        source: &str,
        tool_count: usize,
        bytes: usize,
    ) -> Result<(), String> {
        if *family != NativeModelFamily::DeepSeek || tool_count == 0 {
            return Ok(());
        }
        session
            .record_runtime_event(
                "deepseek.tool_call.assembled",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"source\":{},\"tool_count\":{},\"bytes\":{},\"assembled\":true}}",
                    iteration,
                    json_string(source),
                    tool_count,
                    bytes,
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_continuation_strategy(
        &self,
        session: &mut AgentSession,
        call_id: &str,
        strategy: ContinuationStrategy,
        tool_results: usize,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "model.continuation_strategy",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"strategy\":{},\"tool_results\":{}}}",
                    json_string(call_id),
                    json_string(strategy.event_label()),
                    tool_results
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn select_continuation_strategy(
        &self,
        family: &NativeModelFamily,
        protocol: &str,
    ) -> ContinuationStrategy {
        let _ = (family, protocol);
        ContinuationStrategy::ProviderToolResult
    }

    pub fn should_apply_initial_cache_breakpoints(
        &self,
        family: &NativeModelFamily,
        protocol: &str,
    ) -> bool {
        *family == NativeModelFamily::DeepSeek && protocol == "anthropic_compatible"
    }

    pub fn effective_tool_call_budget(
        &self,
        requested_max_tool_calls: usize,
        long_running: bool,
    ) -> usize {
        if requested_max_tool_calls == 0 {
            u32::MAX as usize
        } else if long_running {
            requested_max_tool_calls.max(64).min(256)
        } else {
            requested_max_tool_calls.min(256)
        }
    }

    pub fn record_tool_call_budget_normalized(
        &self,
        session: &mut AgentSession,
        requested_max_tool_calls: usize,
        effective_max_tool_calls: usize,
    ) -> Result<(), String> {
        if requested_max_tool_calls != 0 {
            return Ok(());
        }
        session
            .record_runtime_event(
                "agent.loop_budget.normalized",
                Actor::Runtime,
                format!(
                    "{{\"requested_max_tool_calls\":0,\"effective_max_tool_calls\":{},\"reason\":\"zero_means_uncapped_native_loop\"}}",
                    effective_max_tool_calls
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn turn_budget_for_request(
        &self,
        max_iterations: usize,
        requested_max_tool_calls: usize,
        max_output_tokens: u64,
        long_running: bool,
    ) -> TurnBudget {
        let effective_max_iterations = if max_iterations == 0 {
            u32::MAX
        } else {
            max_iterations.min(u32::MAX as usize) as u32
        };
        let mut budget = TurnBudget {
            max_iterations: effective_max_iterations,
            max_tool_calls: self
                .effective_tool_call_budget(requested_max_tool_calls, long_running)
                .min(u32::MAX as usize) as u32,
            max_output_tokens,
            ..TurnBudget::default()
        };
        if max_output_tokens > 0 {
            budget.max_output_tokens = max_output_tokens;
        }
        budget
    }

    pub fn record_continuation_plan(
        &self,
        session: &mut AgentSession,
        call_id: &str,
        family: &NativeModelFamily,
        protocol: &str,
        has_raw_reasoning: bool,
        replay_budget_tokens: u64,
        tool_results: usize,
    ) -> Result<ContinuationStrategy, String> {
        let strategy = self.select_continuation_strategy(family, protocol);
        self.record_continuation_strategy(session, call_id, strategy, tool_results)?;
        self.record_reasoning_replay(
            session,
            call_id,
            family,
            protocol,
            strategy,
            has_raw_reasoning,
            replay_budget_tokens,
            tool_results,
        )?;
        Ok(strategy)
    }

    pub fn record_reasoning_replay(
        &self,
        session: &mut AgentSession,
        call_id: &str,
        family: &NativeModelFamily,
        protocol: &str,
        strategy: ContinuationStrategy,
        has_raw_reasoning: bool,
        replay_budget_tokens: u64,
        tool_results: usize,
    ) -> Result<(), String> {
        if *family != NativeModelFamily::DeepSeek
            || protocol != "openai_compatible"
            || strategy == ContinuationStrategy::PlainEvidence
        {
            return Ok(());
        }

        let replay_status = if has_raw_reasoning {
            "provider_reasoning_content"
        } else {
            "placeholder_reasoning_content"
        };
        session
            .record_runtime_event(
                "deepseek.reasoning_replay",
                Actor::Runtime,
                format!(
                    "{{\"call_id\":{},\"status\":{},\"replay_budget_tokens\":{},\"tool_results\":{}}}",
                    json_string(call_id),
                    json_string(replay_status),
                    replay_budget_tokens,
                    tool_results
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_progress_from_observation_cache(
        &self,
        turn_state: &mut TurnState,
        distinct_keys_before: usize,
        recovery_results: u32,
        duplicate_results: u32,
        error_results: u32,
    ) -> ToolProgressReport {
        let (new_evidence_results, decision) = turn_state
            .record_tool_iteration_from_observation_cache(
                distinct_keys_before,
                recovery_results,
                duplicate_results,
                error_results,
            );
        ToolProgressReport {
            new_evidence_results,
            duplicate_results,
            error_results,
            decision,
        }
    }

    pub fn observe_tool_dispatch_progress(
        &mut self,
        session: &mut AgentSession,
        iteration: usize,
        ok_results: usize,
        error_results: usize,
    ) -> Result<(), String> {
        if ok_results == 0 && error_results > 0 {
            self.non_progress_recovery_count += 1;
            session
                .record_runtime_event(
                    "agent.loop_recovery",
                    Actor::Runtime,
                    format!(
                        "{{\"iteration\":{},\"reason\":\"non_progress_iteration\",\"non_progress_count\":{},\"error_results\":{},\"action\":\"continue_with_model_readable_recovery\"}}",
                        iteration,
                        self.non_progress_recovery_count,
                        error_results
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
            if self.non_progress_recovery_count >= 2 {
                session
                    .record_runtime_event(
                        "agent.loop_recovery",
                        Actor::Runtime,
                        format!(
                            "{{\"iteration\":{},\"reason\":\"repeated_non_progress\",\"action\":\"keep_tools_available_until_iteration_budget\"}}",
                            iteration
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
            }
        } else {
            self.non_progress_recovery_count = 0;
        }
        Ok(())
    }

    fn record_turn_convergence_decision(
        &self,
        session: &mut AgentSession,
        turn_state: &mut TurnState,
        iteration: usize,
        source: &'static str,
        verdict: TurnConvergenceVerdict,
        evidence_ref: impl AsRef<str>,
    ) -> Result<(), String> {
        let evidence_ref = evidence_ref.as_ref();
        let disagreement =
            turn_state.record_convergence_decision(iteration, source, verdict, evidence_ref);
        session
            .record_runtime_event(
                "turn.convergence.decision",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"source\":{},\"verdict\":{},\"evidence_ref\":{}}}",
                    iteration,
                    json_string(source),
                    json_string(verdict.as_str()),
                    json_string(evidence_ref)
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        if disagreement {
            session
                .record_runtime_event(
                    "convergence.disagreement",
                    Actor::Runtime,
                    format!(
                        "{{\"iteration\":{},\"source\":{},\"verdict\":{},\"evidence_ref\":{},\"action\":\"record_only\"}}",
                        iteration,
                        json_string(source),
                        json_string(verdict.as_str()),
                        json_string(evidence_ref)
                    ),
                )
                .map_err(|error| format!("{error:?}"))?;
        }
        Ok(())
    }

    pub fn observe_completed_tool_iteration(
        &mut self,
        session: &mut AgentSession,
        turn_state: &mut TurnState,
        convergence_enforcer: &ConvergenceEnforcer,
        evidence_ledger: &EvidenceLedger,
        input: ToolIterationControlInput<'_>,
    ) -> Result<ToolIterationControlAction, String> {
        self.observe_tool_dispatch_progress(
            session,
            input.iteration,
            input.ok_results,
            input.error_results,
        )?;
        let dispatch_verdict = if input.ok_results == 0 && input.error_results > 0 {
            TurnConvergenceVerdict::SoftWarning
        } else {
            TurnConvergenceVerdict::Continue
        };
        self.record_turn_convergence_decision(
            session,
            turn_state,
            input.iteration,
            "non_progress_recovery",
            dispatch_verdict,
            format!(
                "ok_results={} error_results={} non_progress_count={}",
                input.ok_results, input.error_results, self.non_progress_recovery_count
            ),
        )?;
        self.record_duplicate_suppression_summary(
            session,
            input.iteration,
            input.duplicate_results,
        )?;

        let progress_report = self.record_progress_from_observation_cache(
            turn_state,
            input.distinct_keys_before,
            0,
            input.duplicate_results,
            input.progress_error_results,
        );
        let progress_verdict = match progress_report.decision {
            ToolProgressDecision::Continue => TurnConvergenceVerdict::Continue,
            ToolProgressDecision::SoftWarning { .. } => TurnConvergenceVerdict::SoftWarning,
            ToolProgressDecision::Stop { .. } => TurnConvergenceVerdict::Stop,
        };
        self.record_turn_convergence_decision(
            session,
            turn_state,
            input.iteration,
            "ToolProgressState.iteration",
            progress_verdict,
            format!(
                "new_evidence={} duplicates={} errors={}",
                progress_report.new_evidence_results,
                progress_report.duplicate_results,
                progress_report.error_results
            ),
        )?;
        turn_state
            .progress
            .record_successful_tool_results(input.ok_results as u32);

        let unique_error_signatures = input
            .repeated_error_signatures
            .into_iter()
            .collect::<BTreeSet<_>>();
        let contract_error_plateau_decision = if unique_error_signatures.len() == 1 {
            let (tool_id, error_signature) = unique_error_signatures
                .iter()
                .next()
                .expect("unique_error_signatures has one item");
            if error_signature == "SCHEMA_VALIDATION_FAILED" {
                turn_state
                    .progress
                    .record_contract_error_signature(tool_id, error_signature)
            } else {
                turn_state.progress.reset_repeated_contract_error_streak();
                ToolProgressDecision::Continue
            }
        } else {
            turn_state.progress.reset_repeated_contract_error_streak();
            ToolProgressDecision::Continue
        };
        let contract_error_warning_reason = match contract_error_plateau_decision {
            ToolProgressDecision::Stop { reason } => {
                self.record_turn_convergence_decision(
                    session,
                    turn_state,
                    input.iteration,
                    "ToolProgressState.contract_error_signature",
                    TurnConvergenceVerdict::Stop,
                    reason,
                )?;
                self.record_progress_plateau_stopped(
                    session,
                    input.iteration,
                    &progress_report,
                    reason,
                )?;
                return Ok(ToolIterationControlAction::Stop { reason });
            }
            ToolProgressDecision::SoftWarning { reason } => {
                self.record_turn_convergence_decision(
                    session,
                    turn_state,
                    input.iteration,
                    "ToolProgressState.contract_error_signature",
                    TurnConvergenceVerdict::SoftWarning,
                    reason,
                )?;
                self.record_progress_plateau_warned(
                    session,
                    input.iteration,
                    &progress_report,
                    reason,
                )?;
                Some(reason)
            }
            ToolProgressDecision::Continue => None,
        };
        let repeated_error_plateau_decision = if unique_error_signatures.len() == 1 {
            let (tool_id, error_signature) = unique_error_signatures
                .into_iter()
                .next()
                .expect("unique_error_signatures has one item");
            turn_state
                .progress
                .record_error_signature(&tool_id, &error_signature)
        } else {
            if !unique_error_signatures.is_empty() {
                turn_state.progress.reset_repeated_error_streak();
            }
            ToolProgressDecision::Continue
        };

        let repeated_error_warning_reason = match repeated_error_plateau_decision {
            ToolProgressDecision::Stop { reason } => {
                self.record_turn_convergence_decision(
                    session,
                    turn_state,
                    input.iteration,
                    "ToolProgressState.error_signature",
                    TurnConvergenceVerdict::Stop,
                    reason,
                )?;
                self.record_progress_plateau_stopped(
                    session,
                    input.iteration,
                    &progress_report,
                    reason,
                )?;
                return Ok(ToolIterationControlAction::Stop { reason });
            }
            ToolProgressDecision::SoftWarning { reason } => {
                self.record_turn_convergence_decision(
                    session,
                    turn_state,
                    input.iteration,
                    "ToolProgressState.error_signature",
                    TurnConvergenceVerdict::SoftWarning,
                    reason,
                )?;
                self.record_progress_plateau_warned(
                    session,
                    input.iteration,
                    &progress_report,
                    reason,
                )?;
                Some(reason)
            }
            ToolProgressDecision::Continue => None,
        };
        let progress_warning_reason =
            if let ToolProgressDecision::SoftWarning { reason } = progress_report.decision {
                self.record_progress_plateau_warned(
                    session,
                    input.iteration,
                    &progress_report,
                    reason,
                )?;
                Some(reason)
            } else {
                None
            };
        if let ToolProgressDecision::Stop { reason } = progress_report.decision {
            self.record_turn_convergence_decision(
                session,
                turn_state,
                input.iteration,
                "ToolProgressState.iteration",
                TurnConvergenceVerdict::Stop,
                reason,
            )?;
            self.record_progress_plateau_stopped(
                session,
                input.iteration,
                &progress_report,
                reason,
            )?;
            return Ok(ToolIterationControlAction::Stop { reason });
        }

        match self.observe_convergence(
            session,
            convergence_enforcer,
            evidence_ledger,
            input.batch_signature,
            input.seen_tool_batches,
            &progress_report,
            turn_state,
            input.iteration,
        )? {
            LoopConvergenceAction::Continue
                if progress_warning_reason.is_some()
                    || repeated_error_warning_reason.is_some()
                    || contract_error_warning_reason.is_some() =>
            {
                Ok(ToolIterationControlAction::SoftWarning)
            }
            LoopConvergenceAction::Continue => Ok(ToolIterationControlAction::Continue),
            LoopConvergenceAction::SoftWarning { .. } => {
                Ok(ToolIterationControlAction::SoftWarning)
            }
            LoopConvergenceAction::Stop { reason } => {
                Ok(ToolIterationControlAction::Stop { reason })
            }
        }
    }

    pub fn record_progress_plateau_stopped(
        &self,
        session: &mut AgentSession,
        iteration: usize,
        report: &ToolProgressReport,
        reason: &'static str,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.loop_plateau_stopped",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":{},\"new_evidence_results\":{},\"duplicate_results\":{},\"error_results\":{},\"action\":\"stop_with_structured_failure\"}}",
                    iteration,
                    json_string(reason),
                    report.new_evidence_results,
                    report.duplicate_results,
                    report.error_results
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn record_progress_plateau_warned(
        &self,
        session: &mut AgentSession,
        iteration: usize,
        report: &ToolProgressReport,
        reason: &'static str,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.loop_recovery",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":{},\"new_evidence_results\":{},\"duplicate_results\":{},\"error_results\":{},\"action\":\"soft_warning_continue\"}}",
                    iteration,
                    json_string(reason),
                    report.new_evidence_results,
                    report.duplicate_results,
                    report.error_results
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    pub fn observe_convergence(
        &mut self,
        session: &mut AgentSession,
        convergence_enforcer: &ConvergenceEnforcer,
        evidence_ledger: &EvidenceLedger,
        batch_signature: &str,
        seen_tool_batches: &[String],
        progress_report: &ToolProgressReport,
        turn_state: &mut TurnState,
        iteration: usize,
    ) -> Result<LoopConvergenceAction, String> {
        let verdict = convergence_enforcer.observe_iteration(
            evidence_ledger,
            batch_signature,
            seen_tool_batches,
            progress_report.new_evidence_results,
        );
        let turn_verdict = match verdict {
            ConvergenceVerdict::Continue => TurnConvergenceVerdict::Continue,
            ConvergenceVerdict::BatchNoveltyPlateau { .. } => TurnConvergenceVerdict::SoftWarning,
            ConvergenceVerdict::BudgetExhausted => TurnConvergenceVerdict::Stop,
            ConvergenceVerdict::DuplicateDominance { .. }
            | ConvergenceVerdict::InformationStagnation { .. } => {
                TurnConvergenceVerdict::SoftWarning
            }
        };
        self.record_turn_convergence_decision(
            session,
            turn_state,
            iteration,
            "ConvergenceEnforcer",
            turn_verdict,
            format!("{verdict:?}"),
        )?;
        if verdict == ConvergenceVerdict::Continue {
            self.reset_convergence_soft_warning();
            return Ok(LoopConvergenceAction::Continue);
        }

        let reason = convergence_reason(&verdict);
        match verdict {
            ConvergenceVerdict::BudgetExhausted => {
                self.record_convergence_stopped(session, iteration, &verdict, progress_report)?;
                Ok(LoopConvergenceAction::Stop { reason })
            }
            ConvergenceVerdict::BatchNoveltyPlateau { .. } => {
                self.reset_convergence_soft_warning();
                session
                    .record_runtime_event(
                        "agent.loop_recovery",
                        Actor::Runtime,
                        format!(
                            "{{\"iteration\":{},\"reason\":{},\"verdict\":{},\"new_evidence_results\":{},\"duplicate_results\":{},\"error_results\":{},\"action\":\"soft_warning_continue\"}}",
                            iteration,
                            json_string(reason),
                            json_string(&format!("{verdict:?}")),
                            progress_report.new_evidence_results,
                            progress_report.duplicate_results,
                            progress_report.error_results
                        ),
                    )
                    .map_err(|error| format!("{error:?}"))?;
                Ok(LoopConvergenceAction::SoftWarning { reason })
            }
            ConvergenceVerdict::DuplicateDominance { .. }
            | ConvergenceVerdict::InformationStagnation { .. } => self
                .observe_bounded_convergence_plateau(
                    session,
                    iteration,
                    &verdict,
                    progress_report,
                    reason,
                ),
            ConvergenceVerdict::Continue => Ok(LoopConvergenceAction::Continue),
        }
    }

    fn reset_convergence_soft_warning(&mut self) {
        self.convergence_soft_warning_reason = None;
        self.convergence_soft_warning_count = 0;
    }

    fn observe_bounded_convergence_plateau(
        &mut self,
        session: &mut AgentSession,
        iteration: usize,
        verdict: &ConvergenceVerdict,
        progress_report: &ToolProgressReport,
        reason: &'static str,
    ) -> Result<LoopConvergenceAction, String> {
        if self.convergence_soft_warning_reason == Some(reason) {
            self.convergence_soft_warning_count =
                self.convergence_soft_warning_count.saturating_add(1);
        } else {
            self.convergence_soft_warning_reason = Some(reason);
            self.convergence_soft_warning_count = 1;
        }

        if self.convergence_soft_warning_count >= self.max_convergence_soft_warnings {
            self.record_convergence_stopped(session, iteration, verdict, progress_report)?;
            return Ok(LoopConvergenceAction::Stop { reason });
        }

        self.record_convergence_warned(session, iteration, verdict, progress_report)?;
        Ok(LoopConvergenceAction::SoftWarning { reason })
    }

    pub fn reset_loop_guard_recovery(&mut self) {
        self.loop_guard_recovery_count = 0;
    }

    pub fn record_repeated_tool_batch(
        &mut self,
        session: &mut AgentSession,
        iteration: usize,
        alternating_batch: bool,
    ) -> Result<bool, String> {
        self.loop_guard_recovery_count += 1;
        session
            .record_runtime_event(
                "agent.loop_recovery",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":{},\"status\":\"tool_result_recovery\",\"recovery_count\":{},\"max_recoveries\":{}}}",
                    iteration,
                    json_string(if alternating_batch { "alternating_tool_batch" } else { "repeated_tool_batch" }),
                    self.loop_guard_recovery_count,
                    self.max_loop_guard_recoveries
                ),
            )
            .map_err(|error| format!("{error:?}"))?;
        Ok(self.loop_guard_recovery_count > self.max_loop_guard_recoveries)
    }

    pub fn record_repeated_tool_batch_stopped(
        &self,
        session: &mut AgentSession,
        iteration: usize,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.loop_recovery",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":\"repeated_tool_batch_exhausted_recovery\",\"recovery_count\":{},\"max_recoveries\":{},\"action\":\"stop_with_structured_failure\"}}",
                    iteration,
                    self.loop_guard_recovery_count,
                    self.max_loop_guard_recoveries
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    fn record_convergence_stopped(
        &self,
        session: &mut AgentSession,
        iteration: usize,
        verdict: &ConvergenceVerdict,
        progress_report: &ToolProgressReport,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.loop_plateau_stopped",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":{},\"verdict\":{},\"new_evidence_results\":{},\"duplicate_results\":{},\"error_results\":{},\"warning_count\":{},\"max_warnings\":{},\"action\":\"stop_with_structured_failure\"}}",
                    iteration,
                    json_string(convergence_reason(verdict)),
                    json_string(&format!("{verdict:?}")),
                    progress_report.new_evidence_results,
                    progress_report.duplicate_results,
                    progress_report.error_results,
                    self.convergence_soft_warning_count,
                    self.max_convergence_soft_warnings
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }

    fn record_convergence_warned(
        &self,
        session: &mut AgentSession,
        iteration: usize,
        verdict: &ConvergenceVerdict,
        progress_report: &ToolProgressReport,
    ) -> Result<(), String> {
        session
            .record_runtime_event(
                "agent.loop_recovery",
                Actor::Runtime,
                format!(
                    "{{\"iteration\":{},\"reason\":{},\"verdict\":{},\"new_evidence_results\":{},\"duplicate_results\":{},\"error_results\":{},\"warning_count\":{},\"max_warnings\":{},\"action\":\"soft_warning_continue\",\"next_turn_hint\":\"request_permissions_or_change_strategy_without_manifest_mutation\"}}",
                    iteration,
                    json_string(convergence_reason(verdict)),
                    json_string(&format!("{verdict:?}")),
                    progress_report.new_evidence_results,
                    progress_report.duplicate_results,
                    progress_report.error_results,
                    self.convergence_soft_warning_count,
                    self.max_convergence_soft_warnings
                ),
            )
            .map_err(|error| format!("{error:?}"))
    }
}

impl TurnController<NativeLoopIterationContext<'_>> for NativeLoopTurnController {
    fn run_iteration(&mut self, ctx: &mut NativeLoopIterationContext<'_>) -> IterationOutcome {
        match self.begin_iteration(
            ctx.session,
            ctx.turn_state,
            ctx.iteration,
            ctx.tool_call_count,
            ctx.effective_max_tool_calls,
            ctx.has_last_tool_batch,
            ctx.interrupt,
        ) {
            Ok(IterationPreflight::Continue(ids)) => IterationOutcome::Continue { ids },
            Ok(IterationPreflight::Interrupted) => IterationOutcome::Interrupted,
            Ok(IterationPreflight::ToolLimitFailed) => IterationOutcome::Block {
                reason: "max_tool_calls_without_evidence".to_string(),
            },
            Ok(IterationPreflight::ToolLimitStop { reason }) => IterationOutcome::Stop {
                reason: LoopStopReason::ProgressPlateau(reason),
            },
            Err(error) => {
                ctx.error = Some(error.clone());
                IterationOutcome::Block { reason: error }
            }
        }
    }
}

fn convergence_reason(verdict: &ConvergenceVerdict) -> &'static str {
    match verdict {
        ConvergenceVerdict::Continue => "continue",
        ConvergenceVerdict::DuplicateDominance { .. } => "duplicate_dominated_plateau",
        ConvergenceVerdict::InformationStagnation { .. } => "information_stagnation_plateau",
        ConvergenceVerdict::BatchNoveltyPlateau { .. } => "batch_novelty_plateau",
        ConvergenceVerdict::BudgetExhausted => "convergence_budget_exhausted",
    }
}

fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_kernel::{EvidenceClass, IterationProgress};
    use std::sync::atomic::AtomicBool;

    #[test]
    fn iteration_ids_match_native_loop_v2_contract() {
        let ids = NativeLoopIterationIds::for_iteration(7);
        assert_eq!(ids.call_id, "native_loop_v2_call_7");
        assert_eq!(ids.stream_id, "native_loop_v2_stream_7");
        assert_eq!(ids.transcript_id, "native_loop_v2_transcript_7");
    }

    #[test]
    fn progress_report_uses_observation_cache_growth() {
        let controller = NativeLoopTurnController::new();
        let mut state = TurnState::new("sess", 0);
        let before = state.observation_cache.distinct_key_count();
        let report = controller.record_progress_from_observation_cache(&mut state, before, 0, 1, 0);
        assert_eq!(report.new_evidence_results, 0);
        assert_eq!(report.duplicate_results, 1);
        assert_eq!(report.decision, ToolProgressDecision::Continue);
    }

    #[test]
    fn continuation_strategy_labels_are_stable() {
        assert_eq!(
            ContinuationStrategy::from_plain_evidence_preference(true).event_label(),
            "plain_evidence_continuation"
        );
        assert_eq!(
            ContinuationStrategy::from_plain_evidence_preference(false).event_label(),
            "provider_tool_result_continuation"
        );
    }

    #[test]
    fn controller_selects_continuation_strategy_from_provider_shape() {
        let controller = NativeLoopTurnController::new();
        assert_eq!(
            controller
                .select_continuation_strategy(&NativeModelFamily::DeepSeek, "anthropic_compatible"),
            ContinuationStrategy::ProviderToolResult
        );
        assert_eq!(
            controller
                .select_continuation_strategy(&NativeModelFamily::DeepSeek, "openai_compatible"),
            ContinuationStrategy::ProviderToolResult
        );
        assert_eq!(
            controller.select_continuation_strategy(&NativeModelFamily::Qwen, "openai_compatible"),
            ContinuationStrategy::ProviderToolResult
        );
    }

    #[test]
    fn controller_selects_initial_cache_breakpoints_for_deepseek_anthropic() {
        let controller = NativeLoopTurnController::new();
        assert!(controller.should_apply_initial_cache_breakpoints(
            &NativeModelFamily::DeepSeek,
            "anthropic_compatible"
        ));
        assert!(!controller.should_apply_initial_cache_breakpoints(
            &NativeModelFamily::DeepSeek,
            "openai_compatible"
        ));
        assert!(!controller.should_apply_initial_cache_breakpoints(
            &NativeModelFamily::Qwen,
            "anthropic_compatible"
        ));
    }

    #[test]
    fn controller_records_continuation_plan_events() {
        let controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let strategy = controller
            .record_continuation_plan(
                &mut session,
                "call_1",
                &NativeModelFamily::DeepSeek,
                "openai_compatible",
                true,
                4096,
                2,
            )
            .unwrap();
        assert_eq!(strategy, ContinuationStrategy::ProviderToolResult);

        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("model.continuation_strategy"));
        assert!(jsonl.contains("provider_tool_result_continuation"));
        assert!(jsonl.contains("deepseek.reasoning_replay"));
        assert!(jsonl.contains("provider_reasoning_content"));
    }

    #[test]
    fn controller_normalizes_effective_tool_call_budget() {
        let controller = NativeLoopTurnController::new();
        assert_eq!(
            controller.effective_tool_call_budget(0, false),
            u32::MAX as usize
        );
        assert_eq!(
            controller.effective_tool_call_budget(0, true),
            u32::MAX as usize
        );
        assert_eq!(controller.effective_tool_call_budget(3, true), 64);
        assert_eq!(controller.effective_tool_call_budget(99, false), 99);
        assert_eq!(controller.effective_tool_call_budget(999, false), 256);
    }

    #[test]
    fn controller_builds_turn_budget_for_native_loop_request() {
        let controller = NativeLoopTurnController::new();
        let budget = controller.turn_budget_for_request(12, 3, 8192, true);
        assert_eq!(budget.max_iterations, 12);
        assert_eq!(budget.max_tool_calls, 64);
        assert_eq!(budget.max_output_tokens, 8192);
        assert_eq!(
            budget.max_input_tokens,
            TurnBudget::default().max_input_tokens
        );
    }

    #[test]
    fn controller_records_zero_tool_call_budget_normalization() {
        let controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        controller
            .record_tool_call_budget_normalized(&mut session, 0, 256)
            .unwrap();
        controller
            .record_tool_call_budget_normalized(&mut session, 8, 8)
            .unwrap();
        let jsonl = session.event_log().export_jsonl();
        assert_eq!(jsonl.matches("agent.loop_budget.normalized").count(), 1);
        assert!(jsonl.contains("\"effective_max_tool_calls\":256"));
    }

    #[test]
    fn controller_selects_post_tool_batch_action() {
        let controller = NativeLoopTurnController::new();
        assert_eq!(
            controller.select_post_tool_batch_action(true, true, true, true),
            PostToolBatchAction::Continue
        );
        assert_eq!(
            controller.select_post_tool_batch_action(false, true, true, true),
            PostToolBatchAction::Continue
        );
        assert_eq!(
            controller.select_post_tool_batch_action(true, false, true, false),
            PostToolBatchAction::Continue
        );
    }

    #[test]
    fn controller_records_max_iterations_budget_from_turn_budget() {
        let controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        controller
            .record_max_iterations_budget_reached(&mut session, 17)
            .unwrap();
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("agent.loop_budget_reached"));
        assert!(jsonl.contains("\"reason\":\"max_iterations\""));
        assert!(jsonl.contains("\"max_iterations\":17"));
    }

    #[test]
    fn controller_records_deepseek_visible_tool_batch_assembly() {
        let controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        controller
            .record_visible_tool_call_batch_assembled(
                &mut session,
                &NativeModelFamily::DeepSeek,
                3,
                "visible_content_parse",
                2,
                128,
            )
            .unwrap();
        controller
            .record_visible_tool_call_batch_assembled(
                &mut session,
                &NativeModelFamily::Qwen,
                3,
                "visible_content_parse",
                2,
                128,
            )
            .unwrap();
        let jsonl = session.event_log().export_jsonl();
        assert_eq!(jsonl.matches("deepseek.tool_call.assembled").count(), 1);
        assert!(jsonl.contains("\"tool_count\":2"));
        assert!(jsonl.contains("\"bytes\":128"));
    }

    #[test]
    fn controller_canonicalizes_tool_batch_signatures() {
        let controller = NativeLoopTurnController::new();
        let read_8000 = ParsedToolCall {
            provider_tool_call_id: Some("call_a".to_string()),
            tool_id: "file_read".to_string(),
            arguments_json: r#"{"path":"src/lib.rs","max_bytes":8000}"#.to_string(),
            syntax: crate::tcml::ToolCallSyntax::NativeJson,
            status: crate::tcml::ToolCallParseStatus::Parsed,
            repair_applied: false,
        };
        let read_8192 = ParsedToolCall {
            provider_tool_call_id: Some("call_b".to_string()),
            tool_id: "file.read".to_string(),
            arguments_json: r#"{"path":"src/lib.rs","max_bytes":8192}"#.to_string(),
            syntax: crate::tcml::ToolCallSyntax::NativeJson,
            status: crate::tcml::ToolCallParseStatus::Parsed,
            repair_applied: false,
        };

        assert_eq!(
            controller.tool_batch_signature(&[read_8000]),
            controller.tool_batch_signature(&[read_8192])
        );
    }

    #[test]
    fn run_iteration_owns_preflight_gate() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let interrupt = AtomicBool::new(false);
        let mut ctx = NativeLoopIterationContext {
            session: &mut session,
            turn_state: &mut state,
            iteration: 4,
            tool_call_count: 1,
            effective_max_tool_calls: 8,
            has_last_tool_batch: true,
            interrupt: &interrupt,
            error: None,
        };
        assert_eq!(
            controller.run_iteration(&mut ctx),
            IterationOutcome::Continue {
                ids: NativeLoopIterationIds::for_iteration(4)
            }
        );
        assert_eq!(ctx.turn_state.iterations, 5);
        assert_eq!(ctx.turn_state.tool_calls_used, 1);
    }

    #[test]
    fn tool_batch_signature_detects_repeated_and_alternating_batches() {
        let controller = NativeLoopTurnController::new();
        let seen = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        assert_eq!(
            controller.classify_tool_batch_signature(&seen, "gamma"),
            ToolBatchSignatureStatus {
                repeated_tool_batch: true,
                alternating_batch: false,
            }
        );
        assert_eq!(
            controller.classify_tool_batch_signature(&seen, "alpha"),
            ToolBatchSignatureStatus {
                repeated_tool_batch: true,
                alternating_batch: false,
            }
        );
        assert_eq!(
            controller.classify_tool_batch_signature(&seen, "delta"),
            ToolBatchSignatureStatus {
                repeated_tool_batch: false,
                alternating_batch: false,
            }
        );
    }

    #[test]
    fn controller_stops_repeated_batch_guard_after_bounded_recovery() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let repeated = ToolBatchSignatureStatus {
            repeated_tool_batch: true,
            alternating_batch: false,
        };

        assert_eq!(
            controller
                .observe_tool_batch_guard(
                    &mut session,
                    &mut state,
                    0,
                    "batch-a".to_string(),
                    repeated,
                    false,
                )
                .unwrap(),
            ToolBatchGuardAction::UseSyntheticRecovery
        );
        assert_eq!(
            controller
                .observe_tool_batch_guard(
                    &mut session,
                    &mut state,
                    1,
                    "batch-a".to_string(),
                    repeated,
                    false,
                )
                .unwrap(),
            ToolBatchGuardAction::UseSyntheticRecovery
        );
        assert_eq!(
            controller
                .observe_tool_batch_guard(
                    &mut session,
                    &mut state,
                    2,
                    "batch-a".to_string(),
                    repeated,
                    false,
                )
                .unwrap(),
            ToolBatchGuardAction::Stop {
                reason: "repeated_tool_batch_exhausted_recovery"
            }
        );
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("repeated_tool_batch_exhausted_recovery"));
        assert!(jsonl.contains("stop_with_structured_failure"));
        assert!(state.convergence_decisions.iter().any(|decision| {
            decision.source == "NativeLoopTurnController.tool_batch_guard"
                && decision.verdict == TurnConvergenceVerdict::Stop
        }));
    }

    #[test]
    fn repeated_cached_observation_batches_do_not_exhaust_recovery() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let repeated = ToolBatchSignatureStatus {
            repeated_tool_batch: true,
            alternating_batch: false,
        };

        for iteration in 0..5 {
            assert_eq!(
                controller
                    .observe_tool_batch_guard(
                        &mut session,
                        &mut state,
                        iteration,
                        "cached-read-batch".to_string(),
                        repeated,
                        true,
                    )
                    .unwrap(),
                ToolBatchGuardAction::Continue
            );
            assert_eq!(controller.loop_guard_recovery_count(), 0);
        }

        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("duplicate_observation_suppression"));
        assert!(!jsonl.contains("repeated_tool_batch_exhausted_recovery"));
        assert!(!jsonl.contains("stop_with_structured_failure"));
    }

    #[test]
    fn controller_observes_completed_iteration_same_error_plateau_as_soft_warning() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let ledger = EvidenceLedger::default();

        for iteration in 0..2 {
            assert_eq!(
                controller
                    .observe_completed_tool_iteration(
                        &mut session,
                        &mut state,
                        &convergence,
                        &ledger,
                        ToolIterationControlInput {
                            iteration,
                            ok_results: 0,
                            error_results: 1,
                            duplicate_results: 0,
                            distinct_keys_before: 0,
                            progress_error_results: 1,
                            repeated_error_signatures: vec![(
                                "file.read".to_string(),
                                "path_not_found:A.swift".to_string(),
                            )],
                            batch_signature: "batch-a",
                            seen_tool_batches: &[],
                        },
                    )
                    .unwrap(),
                ToolIterationControlAction::Continue
            );
        }

        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration: 2,
                        ok_results: 0,
                        error_results: 1,
                        duplicate_results: 0,
                        distinct_keys_before: 0,
                        progress_error_results: 1,
                        repeated_error_signatures: vec![(
                            "file.read".to_string(),
                            "path_not_found:A.swift".to_string(),
                        )],
                        batch_signature: "batch-a",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::SoftWarning
        );
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("turn.convergence.decision"));
        assert!(jsonl.contains("same_tool_error_plateau"));
        assert!(jsonl.contains("\"verdict\":\"soft_warning\""));
        assert!(jsonl.contains("soft_warning_continue"));
        assert!(!jsonl.contains("agent.loop_plateau_stopped"));
        assert!(state.convergence_decisions.iter().any(|decision| {
            decision.source == "ToolProgressState.error_signature"
                && decision.verdict == TurnConvergenceVerdict::SoftWarning
        }));
    }

    #[test]
    fn controller_counts_repeated_error_signature_once_per_iteration() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let ledger = EvidenceLedger::default();

        for iteration in 0..2 {
            assert_eq!(
                controller
                    .observe_completed_tool_iteration(
                        &mut session,
                        &mut state,
                        &convergence,
                        &ledger,
                        ToolIterationControlInput {
                            iteration,
                            ok_results: 0,
                            error_results: 4,
                            duplicate_results: 0,
                            distinct_keys_before: 0,
                            progress_error_results: 4,
                            repeated_error_signatures: vec![
                                (
                                    "shell.command".to_string(),
                                    "exit_status:swift-test".to_string(),
                                ),
                                (
                                    "shell.command".to_string(),
                                    "exit_status:swift-test".to_string(),
                                ),
                                (
                                    "shell.command".to_string(),
                                    "exit_status:swift-test".to_string(),
                                ),
                                (
                                    "shell.command".to_string(),
                                    "exit_status:swift-test".to_string(),
                                ),
                            ],
                            batch_signature: "batch-a",
                            seen_tool_batches: &[],
                        },
                    )
                    .unwrap(),
                ToolIterationControlAction::Continue
            );
        }

        assert_eq!(state.progress.repeated_error_streak, 2);
        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration: 2,
                        ok_results: 0,
                        error_results: 4,
                        duplicate_results: 0,
                        distinct_keys_before: 0,
                        progress_error_results: 4,
                        repeated_error_signatures: vec![
                            (
                                "shell.command".to_string(),
                                "exit_status:swift-test".to_string(),
                            ),
                            (
                                "shell.command".to_string(),
                                "exit_status:swift-test".to_string(),
                            ),
                        ],
                        batch_signature: "batch-a",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::SoftWarning
        );

        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("same_tool_error_plateau"));
        assert!(jsonl.contains("\"verdict\":\"soft_warning\""));
        assert!(!jsonl.contains("agent.loop_plateau_stopped"));
    }

    #[test]
    fn controller_mixed_success_same_error_resets_streak_without_hard_stop() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let ledger = EvidenceLedger::default();

        for iteration in 0..2 {
            assert_eq!(
                controller
                    .observe_completed_tool_iteration(
                        &mut session,
                        &mut state,
                        &convergence,
                        &ledger,
                        ToolIterationControlInput {
                            iteration,
                            ok_results: 0,
                            error_results: 1,
                            duplicate_results: 0,
                            distinct_keys_before: 0,
                            progress_error_results: 1,
                            repeated_error_signatures: vec![(
                                "file.read".to_string(),
                                "path_not_found:A.swift".to_string(),
                            )],
                            batch_signature: "batch-a",
                            seen_tool_batches: &[],
                        },
                    )
                    .unwrap(),
                ToolIterationControlAction::Continue
            );
        }
        assert_eq!(state.progress.repeated_error_streak, 2);

        assert_ne!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration: 2,
                        ok_results: 1,
                        error_results: 1,
                        duplicate_results: 0,
                        distinct_keys_before: 0,
                        progress_error_results: 1,
                        repeated_error_signatures: vec![(
                            "file.read".to_string(),
                            "path_not_found:A.swift".to_string(),
                        )],
                        batch_signature: "batch-a",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::Stop {
                reason: "same_tool_error_plateau"
            }
        );
        assert_eq!(state.progress.repeated_error_streak, 1);

        let jsonl = session.event_log().export_jsonl();
        assert!(!jsonl.contains("agent.loop_plateau_stopped"));
        assert!(!state.convergence_decisions.iter().any(|decision| {
            decision.source == "ToolProgressState.error_signature"
                && decision.verdict == TurnConvergenceVerdict::Stop
        }));
    }

    #[test]
    fn controller_stops_repeated_schema_validation_errors_even_with_successes() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let ledger = EvidenceLedger::default();

        for iteration in 0..5 {
            let action = controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration,
                        ok_results: 1,
                        error_results: 1,
                        duplicate_results: 0,
                        distinct_keys_before: 0,
                        progress_error_results: 1,
                        repeated_error_signatures: vec![(
                            "file.read".to_string(),
                            "SCHEMA_VALIDATION_FAILED".to_string(),
                        )],
                        batch_signature: "mixed-read-and-schema-error",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap();
            if iteration < 2 {
                assert_eq!(action, ToolIterationControlAction::Continue);
            } else {
                assert_eq!(action, ToolIterationControlAction::SoftWarning);
            }
        }

        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration: 5,
                        ok_results: 1,
                        error_results: 1,
                        duplicate_results: 0,
                        distinct_keys_before: 0,
                        progress_error_results: 1,
                        repeated_error_signatures: vec![(
                            "file.read".to_string(),
                            "SCHEMA_VALIDATION_FAILED".to_string(),
                        )],
                        batch_signature: "mixed-read-and-schema-error",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::Stop {
                reason: "tool_contract_error_plateau"
            }
        );

        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("tool_contract_error_plateau"));
        assert!(jsonl.contains("ToolProgressState.contract_error_signature"));
        assert!(jsonl.contains("agent.loop_plateau_stopped"));
    }

    #[test]
    fn streaming_batch_ready_preserves_pre_append_mismatch_action() {
        let controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        controller
            .record_streaming_batch_ready(&mut session, 0, 2, 2, 1, true)
            .unwrap();
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("continue_with_streamed_results_size_mismatch"));
    }

    #[test]
    fn empty_visible_recovery_count_is_controller_owned() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        controller
            .record_empty_visible_recovery(&mut session, 0)
            .unwrap();
        controller
            .record_empty_visible_recovery(&mut session, 1)
            .unwrap();
        assert_eq!(controller.empty_visible_recovery_count(), 2);
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("\"recovery_count\":2"));
    }

    #[test]
    fn executable_dsml_fallback_records_deepseek_leak_and_escalation() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        for iteration in 0..3 {
            controller
                .record_executable_dsml_fallback(
                    &mut session,
                    iteration,
                    42,
                    &NativeModelFamily::DeepSeek,
                )
                .unwrap();
        }
        assert_eq!(controller.dsml_leak_count(), 3);
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("\"event_type\":\"deepseek.dsml.leak\""));
        assert!(jsonl.contains("\"event_type\":\"deepseek.dsml.leak_escalated\""));
    }

    #[test]
    fn repeated_batch_recovery_exhausts_after_configured_limit() {
        let mut controller = NativeLoopTurnController::new();
        assert_eq!(controller.max_loop_guard_recoveries(), 2);
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        assert!(!controller
            .record_repeated_tool_batch(&mut session, 0, false)
            .unwrap());
        assert!(!controller
            .record_repeated_tool_batch(&mut session, 1, true)
            .unwrap());
        assert!(controller
            .record_repeated_tool_batch(&mut session, 2, false)
            .unwrap());
    }

    #[test]
    fn non_progress_recovery_is_controller_owned() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        controller
            .observe_tool_dispatch_progress(&mut session, 0, 0, 1)
            .unwrap();
        assert_eq!(controller.non_progress_recovery_count(), 1);
        controller
            .observe_tool_dispatch_progress(&mut session, 1, 0, 1)
            .unwrap();
        assert_eq!(controller.non_progress_recovery_count(), 2);
        controller
            .observe_tool_dispatch_progress(&mut session, 2, 1, 0)
            .unwrap();
        assert_eq!(controller.non_progress_recovery_count(), 0);
    }

    #[test]
    fn interrupted_preflight_does_not_mutate_iteration() {
        let controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let interrupt = AtomicBool::new(true);
        assert_eq!(
            controller
                .begin_iteration(&mut session, &mut state, 3, 0, 8, false, &interrupt)
                .unwrap(),
            IterationPreflight::Interrupted
        );
        assert_eq!(state.iterations, 0);
    }

    #[test]
    fn tool_progress_state_warns_duplicate_plateau_without_stopping() {
        let mut state = TurnState::new("session", 0);
        assert_eq!(
            state.progress.record_iteration(IterationProgress {
                new_observation_keys: 0,
                recovery_results: 0,
                duplicate_results: 1,
                error_results: 0,
            }),
            ToolProgressDecision::Continue
        );
        assert_eq!(
            state.progress.record_iteration(IterationProgress {
                new_observation_keys: 0,
                recovery_results: 0,
                duplicate_results: 1,
                error_results: 0,
            }),
            ToolProgressDecision::Continue
        );
        assert_eq!(
            state.progress.record_iteration(IterationProgress {
                new_observation_keys: 0,
                recovery_results: 0,
                duplicate_results: 1,
                error_results: 0,
            }),
            ToolProgressDecision::SoftWarning {
                reason: "duplicate_tool_observation_plateau"
            }
        );
    }

    #[test]
    fn controller_warns_duplicate_plateau_without_stopping_iteration() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let ledger = EvidenceLedger::default();

        for iteration in 0..2 {
            assert_eq!(
                controller
                    .observe_completed_tool_iteration(
                        &mut session,
                        &mut state,
                        &convergence,
                        &ledger,
                        ToolIterationControlInput {
                            iteration,
                            ok_results: 1,
                            error_results: 0,
                            duplicate_results: 1,
                            distinct_keys_before: 0,
                            progress_error_results: 0,
                            repeated_error_signatures: vec![],
                            batch_signature: "batch-a",
                            seen_tool_batches: &[],
                        },
                    )
                    .unwrap(),
                ToolIterationControlAction::Continue
            );
        }

        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration: 2,
                        ok_results: 1,
                        error_results: 0,
                        duplicate_results: 1,
                        distinct_keys_before: 0,
                        progress_error_results: 0,
                        repeated_error_signatures: vec![],
                        batch_signature: "batch-a",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::SoftWarning
        );

        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("duplicate_tool_observation_plateau"));
        assert!(jsonl.contains("soft_warning_continue"));
        assert!(!jsonl.contains("agent.loop_plateau_stopped"));
    }

    #[test]
    fn controller_stops_duplicate_tool_observation_plateau_after_bounded_warnings() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let ledger = EvidenceLedger::default();

        for iteration in 0..5 {
            let expected = if iteration < 2 {
                ToolIterationControlAction::Continue
            } else {
                ToolIterationControlAction::SoftWarning
            };
            assert_eq!(
                controller
                    .observe_completed_tool_iteration(
                        &mut session,
                        &mut state,
                        &convergence,
                        &ledger,
                        ToolIterationControlInput {
                            iteration,
                            ok_results: 1,
                            error_results: 0,
                            duplicate_results: 1,
                            distinct_keys_before: 0,
                            progress_error_results: 0,
                            repeated_error_signatures: vec![],
                            batch_signature: "batch-a",
                            seen_tool_batches: &[],
                        },
                    )
                    .unwrap(),
                expected
            );
        }

        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration: 5,
                        ok_results: 1,
                        error_results: 0,
                        duplicate_results: 1,
                        distinct_keys_before: 0,
                        progress_error_results: 0,
                        repeated_error_signatures: vec![],
                        batch_signature: "batch-a",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::Stop {
                reason: "duplicate_tool_observation_plateau"
            }
        );

        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("duplicate_tool_observation_plateau"));
        assert!(jsonl.contains("agent.loop_plateau_stopped"));
        assert!(jsonl.contains("stop_with_structured_failure"));
    }

    #[test]
    fn convergence_duplicate_dominance_does_not_mutate_tool_manifest() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let mut ledger = EvidenceLedger::default();
        ledger.begin_iteration(0);
        ledger.record_suppressed();
        ledger.begin_iteration(1);
        ledger.record_suppressed();

        let action = controller
            .observe_completed_tool_iteration(
                &mut session,
                &mut state,
                &convergence,
                &ledger,
                ToolIterationControlInput {
                    iteration: 1,
                    ok_results: 0,
                    error_results: 0,
                    duplicate_results: 1,
                    distinct_keys_before: 0,
                    progress_error_results: 0,
                    repeated_error_signatures: vec![],
                    batch_signature: "batch-a",
                    seen_tool_batches: &[],
                },
            )
            .unwrap();

        assert_eq!(action, ToolIterationControlAction::SoftWarning);
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("duplicate_dominated_plateau"));
        assert!(jsonl.contains("soft_warning_continue"));
        assert!(jsonl.contains("request_permissions_or_change_strategy_without_manifest_mutation"));
        assert!(!jsonl.contains("agent.convergence_escalation"));
        assert!(!jsonl.contains("escalate_to_code_edit"));
        assert!(!jsonl.contains("tool.manifest.generated"));
        assert!(state.convergence_decisions.iter().any(|decision| {
            decision.source == "ConvergenceEnforcer"
                && decision.verdict == TurnConvergenceVerdict::SoftWarning
        }));
    }

    #[test]
    fn convergence_duplicate_dominance_stops_after_bounded_warnings() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let mut ledger = EvidenceLedger::default();
        ledger.begin_iteration(0);
        ledger.record_suppressed();
        ledger.begin_iteration(1);
        ledger.record_suppressed();

        for iteration in 1..3 {
            assert_eq!(
                controller
                    .observe_completed_tool_iteration(
                        &mut session,
                        &mut state,
                        &convergence,
                        &ledger,
                        ToolIterationControlInput {
                            iteration,
                            ok_results: 0,
                            error_results: 0,
                            duplicate_results: 1,
                            distinct_keys_before: 0,
                            progress_error_results: 0,
                            repeated_error_signatures: vec![],
                            batch_signature: "batch-a",
                            seen_tool_batches: &[],
                        },
                    )
                    .unwrap(),
                ToolIterationControlAction::SoftWarning
            );
        }

        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &ledger,
                    ToolIterationControlInput {
                        iteration: 3,
                        ok_results: 0,
                        error_results: 0,
                        duplicate_results: 1,
                        distinct_keys_before: 0,
                        progress_error_results: 0,
                        repeated_error_signatures: vec![],
                        batch_signature: "batch-a",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::Stop {
                reason: "duplicate_dominated_plateau"
            }
        );
        assert_eq!(controller.convergence_soft_warning_count(), 3);
        let jsonl = session.event_log().export_jsonl();
        assert!(jsonl.contains("duplicate_dominated_plateau"));
        assert!(jsonl.contains("\"warning_count\":3"));
        assert!(jsonl.contains("agent.loop_plateau_stopped"));
        assert!(jsonl.contains("stop_with_structured_failure"));
        assert!(!jsonl.contains("agent.convergence_escalation"));
        assert!(!jsonl.contains("escalate_to_code_edit"));
    }

    #[test]
    fn convergence_warning_count_resets_after_new_evidence() {
        let mut controller = NativeLoopTurnController::new();
        let mut session = AgentSession::new("project", "session", "task").unwrap();
        let mut state = TurnState::new("session", 0);
        let convergence = ConvergenceEnforcer::default();
        let mut duplicate_ledger = EvidenceLedger::default();
        duplicate_ledger.begin_iteration(0);
        duplicate_ledger.record_suppressed();
        duplicate_ledger.begin_iteration(1);
        duplicate_ledger.record_suppressed();

        for iteration in 1..3 {
            assert_eq!(
                controller
                    .observe_completed_tool_iteration(
                        &mut session,
                        &mut state,
                        &convergence,
                        &duplicate_ledger,
                        ToolIterationControlInput {
                            iteration,
                            ok_results: 0,
                            error_results: 0,
                            duplicate_results: 1,
                            distinct_keys_before: 0,
                            progress_error_results: 0,
                            repeated_error_signatures: vec![],
                            batch_signature: "batch-a",
                            seen_tool_batches: &[],
                        },
                    )
                    .unwrap(),
                ToolIterationControlAction::SoftWarning
            );
        }
        assert_eq!(controller.convergence_soft_warning_count(), 2);

        let mut fresh_ledger = EvidenceLedger::default();
        fresh_ledger.begin_iteration(2);
        fresh_ledger.push(
            "toolu_new".to_string(),
            "file.read".to_string(),
            "{\"path\":\"README.md\"}".to_string(),
            crate::tool_execution::ToolExecutionResult {
                tool_call_id: "toolu_new".to_string(),
                tool_id: "file.read".to_string(),
                ok: true,
                preview: "new evidence".to_string(),
                detail_json: "{}".to_string(),
                exit_code: None,
            },
            EvidenceClass::NewEvidence,
        );
        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &fresh_ledger,
                    ToolIterationControlInput {
                        iteration: 3,
                        ok_results: 1,
                        error_results: 0,
                        duplicate_results: 0,
                        distinct_keys_before: 0,
                        progress_error_results: 0,
                        repeated_error_signatures: vec![],
                        batch_signature: "batch-new",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::Continue
        );
        assert_eq!(controller.convergence_soft_warning_count(), 0);

        assert_eq!(
            controller
                .observe_completed_tool_iteration(
                    &mut session,
                    &mut state,
                    &convergence,
                    &duplicate_ledger,
                    ToolIterationControlInput {
                        iteration: 4,
                        ok_results: 0,
                        error_results: 0,
                        duplicate_results: 1,
                        distinct_keys_before: 0,
                        progress_error_results: 0,
                        repeated_error_signatures: vec![],
                        batch_signature: "batch-a",
                        seen_tool_batches: &[],
                    },
                )
                .unwrap(),
            ToolIterationControlAction::SoftWarning
        );
        assert_eq!(controller.convergence_soft_warning_count(), 1);
    }
}
