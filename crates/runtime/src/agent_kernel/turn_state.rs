use std::time::Instant;

use crate::agent_kernel::ObservationCache;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnRoute {
    DirectAnswer,
    ProjectStatus,
    ReadOnlyExplore,
    CodeEdit,
    DebugFailure,
    RunTests,
    LongHorizonTask,
    Review,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentRole {
    Executor,
    Compactor,
    Reviewer,
    Titler,
    Summarizer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnBudget {
    pub max_iterations: u32,
    pub max_tool_calls: u32,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_reasoning_tokens: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolProgressDecision {
    Continue,
    SoftWarning { reason: &'static str },
    Stop { reason: &'static str },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnConvergenceVerdict {
    Continue,
    SoftWarning,
    Stop,
}

impl TurnConvergenceVerdict {
    pub fn as_str(self) -> &'static str {
        match self {
            TurnConvergenceVerdict::Continue => "continue",
            TurnConvergenceVerdict::SoftWarning => "soft_warning",
            TurnConvergenceVerdict::Stop => "stop",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnConvergenceDecision {
    pub iteration: u32,
    pub source: String,
    pub verdict: TurnConvergenceVerdict,
    pub evidence_ref: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IterationProgress {
    pub new_observation_keys: u32,
    pub recovery_results: u32,
    pub duplicate_results: u32,
    pub error_results: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolProgressState {
    pub new_observation_keys: u32,
    pub recovery_results: u32,
    pub duplicate_results: u32,
    pub error_results: u32,
    pub consecutive_no_progress_iterations: u32,
    pub consecutive_duplicate_iterations: u32,
    pub repeated_error_signature: Option<String>,
    pub repeated_error_streak: u32,
    pub repeated_contract_error_signature: Option<String>,
    pub repeated_contract_error_streak: u32,
}

const DUPLICATE_TOOL_OBSERVATION_PLATEAU_ITERATIONS: u32 = 3;
const DUPLICATE_TOOL_OBSERVATION_PLATEAU_STOP_ITERATIONS: u32 = 6;
const NON_PROGRESS_TOOL_PLATEAU_ITERATIONS: u32 = 3;
const NON_PROGRESS_TOOL_PLATEAU_STOP_ITERATIONS: u32 = 6;
const SAME_TOOL_ERROR_SOFT_WARNING_STREAK: u32 = 3;
const SAME_TOOL_ERROR_STOP_STREAK: u32 = 6;
const CONTRACT_ERROR_SOFT_WARNING_STREAK: u32 = 3;
const CONTRACT_ERROR_STOP_STREAK: u32 = 6;

impl ToolProgressState {
    pub fn record_error_signature(
        &mut self,
        tool_id: &str,
        error_code: &str,
    ) -> ToolProgressDecision {
        let signature = format!("{tool_id}:{error_code}");
        if self.repeated_error_signature.as_deref() == Some(signature.as_str()) {
            self.repeated_error_streak = self.repeated_error_streak.saturating_add(1);
        } else {
            self.repeated_error_signature = Some(signature);
            self.repeated_error_streak = 1;
        }

        if self.repeated_error_streak >= SAME_TOOL_ERROR_STOP_STREAK {
            ToolProgressDecision::Stop {
                reason: "same_tool_error_plateau",
            }
        } else if self.repeated_error_streak >= SAME_TOOL_ERROR_SOFT_WARNING_STREAK {
            ToolProgressDecision::SoftWarning {
                reason: "same_tool_error_plateau",
            }
        } else {
            ToolProgressDecision::Continue
        }
    }

    pub fn record_contract_error_signature(
        &mut self,
        tool_id: &str,
        error_code: &str,
    ) -> ToolProgressDecision {
        let signature = format!("{tool_id}:{error_code}");
        if self.repeated_contract_error_signature.as_deref() == Some(signature.as_str()) {
            self.repeated_contract_error_streak =
                self.repeated_contract_error_streak.saturating_add(1);
        } else {
            self.repeated_contract_error_signature = Some(signature);
            self.repeated_contract_error_streak = 1;
        }

        if self.repeated_contract_error_streak >= CONTRACT_ERROR_STOP_STREAK {
            ToolProgressDecision::Stop {
                reason: "tool_contract_error_plateau",
            }
        } else if self.repeated_contract_error_streak >= CONTRACT_ERROR_SOFT_WARNING_STREAK {
            ToolProgressDecision::SoftWarning {
                reason: "tool_contract_error_plateau",
            }
        } else {
            ToolProgressDecision::Continue
        }
    }

    pub fn reset_repeated_contract_error_streak(&mut self) {
        self.repeated_contract_error_signature = None;
        self.repeated_contract_error_streak = 0;
    }

    pub fn record_successful_tool_results(&mut self, ok_results: u32) {
        if ok_results > 0 {
            self.reset_repeated_error_streak();
        }
    }

    pub fn reset_repeated_error_streak(&mut self) {
        self.repeated_error_signature = None;
        self.repeated_error_streak = 0;
    }

    pub fn record_iteration(&mut self, progress: IterationProgress) -> ToolProgressDecision {
        self.new_observation_keys = self
            .new_observation_keys
            .saturating_add(progress.new_observation_keys);
        self.recovery_results = self
            .recovery_results
            .saturating_add(progress.recovery_results);
        self.duplicate_results = self
            .duplicate_results
            .saturating_add(progress.duplicate_results);
        self.error_results = self.error_results.saturating_add(progress.error_results);

        if progress.new_observation_keys > 0 {
            self.reset_repeated_error_streak();
            self.consecutive_no_progress_iterations = 0;
            self.consecutive_duplicate_iterations = 0;
            return ToolProgressDecision::Continue;
        }

        if progress.duplicate_results > 0 {
            self.consecutive_duplicate_iterations =
                self.consecutive_duplicate_iterations.saturating_add(1);
        } else {
            self.consecutive_duplicate_iterations = 0;
        }

        if progress.duplicate_results > 0
            || progress.recovery_results > 0
            || progress.error_results > 0
        {
            self.consecutive_no_progress_iterations =
                self.consecutive_no_progress_iterations.saturating_add(1);
        }

        if self.consecutive_duplicate_iterations
            >= DUPLICATE_TOOL_OBSERVATION_PLATEAU_STOP_ITERATIONS
        {
            ToolProgressDecision::Stop {
                reason: "duplicate_tool_observation_plateau",
            }
        } else if self.consecutive_no_progress_iterations
            >= NON_PROGRESS_TOOL_PLATEAU_STOP_ITERATIONS
        {
            ToolProgressDecision::Stop {
                reason: "non_progress_tool_plateau",
            }
        } else if self.consecutive_duplicate_iterations
            >= DUPLICATE_TOOL_OBSERVATION_PLATEAU_ITERATIONS
        {
            ToolProgressDecision::SoftWarning {
                reason: "duplicate_tool_observation_plateau",
            }
        } else if self.consecutive_no_progress_iterations >= NON_PROGRESS_TOOL_PLATEAU_ITERATIONS {
            ToolProgressDecision::SoftWarning {
                reason: "non_progress_tool_plateau",
            }
        } else {
            ToolProgressDecision::Continue
        }
    }
}

impl Default for TurnBudget {
    fn default() -> Self {
        Self {
            max_iterations: 8,
            max_tool_calls: 0,
            max_input_tokens: 192_000,
            max_output_tokens: 16_384,
            max_reasoning_tokens: 64_000,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AwaitingUserRequest {
    PermissionDecision { permission_id: String },
    InterruptedThinking { reason: String },
}

#[derive(Debug, Clone)]
pub struct TurnState {
    pub session_id: String,
    pub turn_index: u32,
    pub started_at: Instant,
    pub route: TurnRoute,
    pub mode: crate::agent_kernel::PermissionMode,
    pub role: AgentRole,
    pub budget: TurnBudget,
    pub iterations: u32,
    pub tool_calls_used: u32,
    pub tokens_in: u64,
    pub tokens_out: u64,
    pub reasoning_tokens: u64,
    pub seen_tool_batches: Vec<String>,
    pub observation_cache: ObservationCache,
    pub progress: ToolProgressState,
    pub convergence_decisions: Vec<TurnConvergenceDecision>,
    pub last_tool_batch: Vec<String>,
    pub emitted_event_count: usize,
    pub awaiting_user: Option<AwaitingUserRequest>,
}

impl TurnState {
    pub fn new(session_id: impl Into<String>, turn_index: u32) -> Self {
        Self {
            session_id: session_id.into(),
            turn_index,
            started_at: Instant::now(),
            route: TurnRoute::ReadOnlyExplore,
            mode: crate::agent_kernel::PermissionMode::Default,
            role: AgentRole::Executor,
            budget: TurnBudget::default(),
            iterations: 0,
            tool_calls_used: 0,
            tokens_in: 0,
            tokens_out: 0,
            reasoning_tokens: 0,
            seen_tool_batches: Vec::new(),
            observation_cache: ObservationCache::default(),
            progress: ToolProgressState::default(),
            convergence_decisions: Vec::new(),
            last_tool_batch: Vec::new(),
            emitted_event_count: 0,
            awaiting_user: None,
        }
    }

    pub fn record_reasoning_tokens(&mut self, tokens: u64) {
        self.reasoning_tokens = self.reasoning_tokens.saturating_add(tokens);
    }

    pub fn record_tool_iteration_from_observation_cache(
        &mut self,
        distinct_keys_before: usize,
        recovery_results: u32,
        duplicate_results: u32,
        error_results: u32,
    ) -> (u32, ToolProgressDecision) {
        let distinct_keys_now = self.observation_cache.distinct_key_count();
        let new_observation_keys = distinct_keys_now.saturating_sub(distinct_keys_before) as u32;
        let decision = self.progress.record_iteration(IterationProgress {
            new_observation_keys,
            recovery_results,
            duplicate_results,
            error_results,
        });
        (new_observation_keys, decision)
    }

    pub fn record_convergence_decision(
        &mut self,
        iteration: usize,
        source: impl Into<String>,
        verdict: TurnConvergenceVerdict,
        evidence_ref: impl Into<String>,
    ) -> bool {
        let iteration = iteration as u32;
        let disagreement = self
            .convergence_decisions
            .iter()
            .any(|decision| decision.iteration == iteration && decision.verdict != verdict);
        self.convergence_decisions.push(TurnConvergenceDecision {
            iteration,
            source: source.into(),
            verdict,
            evidence_ref: evidence_ref.into(),
        });
        disagreement
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_state_tracks_reasoning_tokens_separately() {
        let mut state = TurnState::new("sess", 1);
        state.tokens_out = 10;
        state.record_reasoning_tokens(42);
        assert_eq!(state.tokens_out, 10);
        assert_eq!(state.reasoning_tokens, 42);
    }

    #[test]
    fn progress_state_allows_unbounded_evidence_but_warns_on_plateaus() {
        let mut state = ToolProgressState::default();
        for _ in 0..100 {
            assert_eq!(
                state.record_iteration(IterationProgress {
                    new_observation_keys: 1,
                    recovery_results: 0,
                    duplicate_results: 0,
                    error_results: 0,
                }),
                ToolProgressDecision::Continue
            );
        }
        assert_eq!(state.new_observation_keys, 100);
        assert_eq!(
            state.record_iteration(IterationProgress {
                new_observation_keys: 0,
                recovery_results: 0,
                duplicate_results: 1,
                error_results: 0,
            }),
            ToolProgressDecision::Continue
        );
        assert_eq!(
            state.record_iteration(IterationProgress {
                new_observation_keys: 0,
                recovery_results: 0,
                duplicate_results: 1,
                error_results: 0,
            }),
            ToolProgressDecision::Continue
        );
        assert_eq!(
            state.record_iteration(IterationProgress {
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
    fn progress_state_stops_duplicate_plateau_after_bounded_warnings() {
        let mut state = ToolProgressState::default();
        for _ in 0..2 {
            assert_eq!(
                state.record_iteration(IterationProgress {
                    new_observation_keys: 0,
                    recovery_results: 0,
                    duplicate_results: 1,
                    error_results: 0,
                }),
                ToolProgressDecision::Continue
            );
        }
        for _ in 0..3 {
            assert_eq!(
                state.record_iteration(IterationProgress {
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
        assert_eq!(
            state.record_iteration(IterationProgress {
                new_observation_keys: 0,
                recovery_results: 0,
                duplicate_results: 1,
                error_results: 0,
            }),
            ToolProgressDecision::Stop {
                reason: "duplicate_tool_observation_plateau"
            }
        );
    }

    #[test]
    fn progress_state_stops_non_progress_plateau_after_bounded_warnings() {
        let mut state = ToolProgressState::default();
        for _ in 0..2 {
            assert_eq!(
                state.record_iteration(IterationProgress {
                    new_observation_keys: 0,
                    recovery_results: 1,
                    duplicate_results: 0,
                    error_results: 0,
                }),
                ToolProgressDecision::Continue
            );
        }
        for _ in 0..3 {
            assert_eq!(
                state.record_iteration(IterationProgress {
                    new_observation_keys: 0,
                    recovery_results: 1,
                    duplicate_results: 0,
                    error_results: 0,
                }),
                ToolProgressDecision::SoftWarning {
                    reason: "non_progress_tool_plateau"
                }
            );
        }
        assert_eq!(
            state.record_iteration(IterationProgress {
                new_observation_keys: 0,
                recovery_results: 1,
                duplicate_results: 0,
                error_results: 0,
            }),
            ToolProgressDecision::Stop {
                reason: "non_progress_tool_plateau"
            }
        );
    }

    #[test]
    fn same_tool_error_plateau_warns_at_three_and_stops_at_higher_threshold() {
        let mut state = ToolProgressState::default();
        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::Continue
        );
        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::Continue
        );
        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::SoftWarning {
                reason: "same_tool_error_plateau"
            }
        );
        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::SoftWarning {
                reason: "same_tool_error_plateau"
            }
        );
        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::SoftWarning {
                reason: "same_tool_error_plateau"
            }
        );
        assert!(matches!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::Stop { reason } if reason == "same_tool_error_plateau"
        ));
    }

    #[test]
    fn same_tool_error_streak_resets_after_new_observation_or_success() {
        let mut state = ToolProgressState::default();
        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::Continue
        );
        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::Continue
        );
        assert_eq!(state.repeated_error_streak, 2);

        assert_eq!(
            state.record_iteration(IterationProgress {
                new_observation_keys: 1,
                recovery_results: 0,
                duplicate_results: 0,
                error_results: 0,
            }),
            ToolProgressDecision::Continue
        );
        assert_eq!(state.repeated_error_streak, 0);

        assert_eq!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::Continue
        );
        state.record_successful_tool_results(1);
        assert_eq!(state.repeated_error_streak, 0);

        assert!(matches!(
            state.record_error_signature("file.read", "TOOL_NOT_IN_MANIFEST"),
            ToolProgressDecision::Continue
        ));
    }

    #[test]
    fn contract_error_plateau_is_tracked_separately_from_success_progress() {
        let mut state = ToolProgressState::default();
        for _ in 0..2 {
            assert_eq!(
                state.record_contract_error_signature("file.read", "SCHEMA_VALIDATION_FAILED"),
                ToolProgressDecision::Continue
            );
            state.record_successful_tool_results(1);
        }
        assert_eq!(state.repeated_contract_error_streak, 2);
        assert_eq!(
            state.record_contract_error_signature("file.read", "SCHEMA_VALIDATION_FAILED"),
            ToolProgressDecision::SoftWarning {
                reason: "tool_contract_error_plateau"
            }
        );
        for _ in 0..2 {
            assert_eq!(
                state.record_contract_error_signature("file.read", "SCHEMA_VALIDATION_FAILED"),
                ToolProgressDecision::SoftWarning {
                    reason: "tool_contract_error_plateau"
                }
            );
        }
        assert!(matches!(
            state.record_contract_error_signature("file.read", "SCHEMA_VALIDATION_FAILED"),
            ToolProgressDecision::Stop { reason } if reason == "tool_contract_error_plateau"
        ));
        state.reset_repeated_contract_error_streak();
        assert_eq!(state.repeated_contract_error_streak, 0);
    }

    #[test]
    fn turn_state_progress_uses_observation_cache_growth_not_ok_results() {
        let mut state = TurnState::new("sess", 1);
        let before = state.observation_cache.distinct_key_count();
        assert_eq!(
            state.record_tool_iteration_from_observation_cache(before, 0, 1, 0),
            (0, ToolProgressDecision::Continue)
        );
        assert_eq!(
            state.record_tool_iteration_from_observation_cache(before, 0, 1, 0),
            (0, ToolProgressDecision::Continue)
        );
        assert_eq!(
            state.record_tool_iteration_from_observation_cache(before, 0, 1, 0),
            (
                0,
                ToolProgressDecision::SoftWarning {
                    reason: "duplicate_tool_observation_plateau"
                }
            )
        );
    }

    #[test]
    fn turn_state_records_convergence_disagreement_by_iteration() {
        let mut state = TurnState::new("sess", 1);
        assert!(!state.record_convergence_decision(
            0,
            "ToolProgressState",
            TurnConvergenceVerdict::Continue,
            "new_keys=1",
        ));
        assert!(state.record_convergence_decision(
            0,
            "ConvergenceEnforcer",
            TurnConvergenceVerdict::Stop,
            "duplicate_dominance",
        ));
        assert!(!state.record_convergence_decision(
            1,
            "ConvergenceEnforcer",
            TurnConvergenceVerdict::Stop,
            "duplicate_dominance",
        ));
        assert_eq!(state.convergence_decisions.len(), 3);
    }
}
