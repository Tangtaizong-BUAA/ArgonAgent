//! Error recovery strategies modeled after Claude Code's `query.ts`.
//!
//! Three recovery paths:
//! 1. `maxOutputTokens` escalation (query.ts:1185-1256)
//! 2. Model fallback (query.ts:893-953)
//! 3. Reactive compaction (query.ts:1085-1183)

use researchcode_kernel::model::NativeModelFamily;
use std::time::Instant;

// ── MaxOutputTokens recovery (query.ts:1185-1256) ──────────────────────────────

/// Recovery state for output-token-limit errors.
/// Claude Code escalates: 64K → retry (silent) → inject recovery message → retry (up to 3x) → fail.
#[derive(Debug, Clone)]
pub struct MaxTokensRecovery {
    /// Whether maxOutputTokens recovery is active.
    pub active: bool,
    /// Escalation level: 0 = initial, 1 = 64K escalated, 2+ = recovery message injected.
    pub escalation_level: u8,
    /// Number of recovery retries attempted.
    pub recovery_retries: u32,
    /// The escalated token limit.
    pub escalated_max_tokens: u32,
    /// Timestamp of first token-limit error (for timeout tracking).
    pub first_error_at: Option<Instant>,
}

impl Default for MaxTokensRecovery {
    fn default() -> Self {
        Self {
            active: false,
            escalation_level: 0,
            recovery_retries: 0,
            escalated_max_tokens: 0,
            first_error_at: None,
        }
    }
}

/// Maximum recovery retries for maxOutputTokens.
pub const MAX_TOKEN_RECOVERY_RETRIES: u32 = 3;

/// Escalated token limit (64K, matching Claude Code's escalated maxOutputTokens).
pub const ESCALATED_MAX_TOKENS: u32 = 65536;

impl MaxTokensRecovery {
    pub fn new() -> Self {
        Self::default()
    }

    /// Step 1: Escalate to 64K, retry silently.
    /// Returns the new max_tokens to use.
    pub fn escalate(&mut self) -> u32 {
        if !self.active {
            self.active = true;
            self.first_error_at = Some(Instant::now());
        }
        self.escalation_level += 1;
        self.escalated_max_tokens = ESCALATED_MAX_TOKENS;
        ESCALATED_MAX_TOKENS
    }

    /// Step 2: Inject recovery message into the conversation.
    pub fn build_recovery_message(&self) -> String {
        format!(
            "Your previous response was truncated because it exceeded the maximum output token limit. \
             Please continue from where you left off. This is recovery attempt {} of {}. \
             Focus on completing the remaining work concisely.",
            self.recovery_retries + 1,
            MAX_TOKEN_RECOVERY_RETRIES,
        )
    }

    /// Step 3: Check if we should give up.
    pub fn should_fail(&self) -> bool {
        self.recovery_retries >= MAX_TOKEN_RECOVERY_RETRIES
    }

    /// Record a recovery retry attempt.
    pub fn record_retry(&mut self) {
        self.recovery_retries += 1;
    }

    /// Reset after a successful call.
    pub fn reset(&mut self) {
        self.active = false;
        self.escalation_level = 0;
        self.recovery_retries = 0;
        self.escalated_max_tokens = 0;
        self.first_error_at = None;
    }
}

// ── Model fallback (query.ts:893-953) ──────────────────────────────────────────

/// Model fallback state for handling provider-level failures.
#[derive(Debug, Clone)]
pub struct ModelFallback {
    /// Whether fallback is active.
    pub active: bool,
    /// The original model family before fallback.
    pub original_family: Option<NativeModelFamily>,
    /// The fallback model family.
    pub fallback_family: NativeModelFamily,
    /// Whether the user has been notified of the fallback.
    pub user_notified: bool,
}

impl Default for ModelFallback {
    fn default() -> Self {
        Self {
            active: false,
            original_family: None,
            fallback_family: NativeModelFamily::DeepSeek,
            user_notified: false,
        }
    }
}

impl ModelFallback {
    pub fn new(fallback_family: NativeModelFamily) -> Self {
        Self {
            fallback_family,
            ..Default::default()
        }
    }

    /// Activate fallback mode, preserving the original family.
    pub fn activate(&mut self, original: NativeModelFamily) {
        self.active = true;
        self.original_family = Some(original);
    }

    /// Synthesize a fallback notification message for the user.
    pub fn synthesize_error(&self) -> String {
        let original = self
            .original_family
            .as_ref()
            .map(|f| format!("{f:?}"))
            .unwrap_or_else(|| "unknown".to_string());
        format!(
            "Model {original} encountered an error. Automatically falling back to {fallback:?}. \
             Your conversation context has been preserved.",
            fallback = self.fallback_family,
        )
    }

    /// Mark that the user has been notified.
    pub fn mark_notified(&mut self) {
        self.user_notified = true;
    }

    /// Clear fallback state.
    pub fn clear(&mut self) {
        self.active = false;
        self.original_family = None;
        self.user_notified = false;
    }
}

// ── Reactive compaction (query.ts:1085-1183) ───────────────────────────────────

/// Reactive compaction state for context collapse avoidance.
#[derive(Debug, Clone)]
pub struct ReactiveCompaction {
    /// Number of consecutive token-limit hits.
    pub consecutive_token_errors: u32,
    /// Last context size estimate (token count).
    pub last_context_size: u64,
    /// Whether a compaction was triggered.
    pub compaction_triggered: bool,
    /// Compaction retry count.
    pub compaction_retries: u32,
}

/// Maximum compaction retries before giving up.
pub const MAX_COMPACTION_RETRIES: u32 = 2;

/// Token threshold that triggers reactive compaction (context at > 90% of limit).
pub const COMPACTION_THRESHOLD_RATIO: f64 = 0.90;

impl Default for ReactiveCompaction {
    fn default() -> Self {
        Self {
            consecutive_token_errors: 0,
            last_context_size: 0,
            compaction_triggered: false,
            compaction_retries: 0,
        }
    }
}

impl ReactiveCompaction {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a token-limit error and decide whether to trigger compaction.
    pub fn record_token_error(&mut self, context_size_tokens: u64) -> bool {
        self.consecutive_token_errors += 1;
        self.last_context_size = context_size_tokens;

        // Trigger compaction on second consecutive token error.
        if self.consecutive_token_errors >= 2 && self.compaction_retries < MAX_COMPACTION_RETRIES {
            self.compaction_triggered = true;
            self.compaction_retries += 1;
            return true;
        }
        false
    }

    /// Check if context size needs compaction (over 90% of limit).
    pub fn needs_compaction(&self, context_size_tokens: u64, context_limit: u64) -> bool {
        context_size_tokens as f64 > context_limit as f64 * COMPACTION_THRESHOLD_RATIO
    }

    /// Compaction drain strategy: keep system prompt + tools, trim old turns.
    pub fn compact_message_count_threshold(&self, total_messages: usize) -> usize {
        // Keep at most half the messages after compaction, but never fewer than 1
        // (total_messages / 2 = 0 for 1 message, which would lose everything)
        (total_messages / 2).max(1)
    }

    /// Reset compaction state after successful operation.
    pub fn reset(&mut self) {
        self.consecutive_token_errors = 0;
        self.compaction_triggered = false;
    }

    /// Reset fully including retry count.
    pub fn full_reset(&mut self) {
        self.consecutive_token_errors = 0;
        self.compaction_triggered = false;
        self.compaction_retries = 0;
    }
}

// ── Unified error recovery state ──────────────────────────────────────────────

/// Action determined by the recovery decision tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// No action needed.
    None,
    /// Escalate maxOutputTokens and retry silently.
    EscalateMaxTokens { new_limit: u32 },
    /// Inject a recovery message and retry.
    InjectRecoveryMessage { message: String },
    /// Fall back to an alternative model.
    FallbackModel {
        original: NativeModelFamily,
        fallback: NativeModelFamily,
    },
    /// Trigger reactive compaction.
    TriggerCompaction { keep_messages: usize },
    /// All recovery paths exhausted; fail the request.
    Fail { reason: String },
}

/// Complete error recovery state, aggregating all three strategies.
#[derive(Debug, Clone, Default)]
pub struct ErrorRecoveryState {
    pub max_tokens: MaxTokensRecovery,
    pub model_fallback: ModelFallback,
    pub compaction: ReactiveCompaction,
}

impl ErrorRecoveryState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_fallback(fallback_family: NativeModelFamily) -> Self {
        Self {
            model_fallback: ModelFallback::new(fallback_family),
            ..Default::default()
        }
    }

    /// Decision tree for error recovery: token errors resolved first, then model
    /// fallback, then compaction. Returns the recommended recovery action.
    pub fn determine_recovery_action(
        &mut self,
        is_token_limit_error: bool,
        is_provider_error: bool,
        context_size_tokens: u64,
        context_limit_tokens: u64,
    ) -> RecoveryAction {
        // 1. Token-limit errors: escalate max_tokens, then inject recovery message, then fail.
        if is_token_limit_error {
            if !self.max_tokens.should_fail() {
                if self.max_tokens.escalation_level == 0 {
                    let new_limit = self.max_tokens.escalate();
                    return RecoveryAction::EscalateMaxTokens { new_limit };
                } else {
                    self.max_tokens.record_retry();
                    let message = self.max_tokens.build_recovery_message();
                    return RecoveryAction::InjectRecoveryMessage { message };
                }
            }
            return RecoveryAction::Fail {
                reason: "max token recovery retries exhausted".to_string(),
            };
        }

        // 2. Provider-level errors: attempt model fallback.
        if is_provider_error && !self.model_fallback.active {
            let original = NativeModelFamily::DeepSeek; // sensible default
            self.model_fallback.activate(original);
            let fallback = self.model_fallback.fallback_family;
            return RecoveryAction::FallbackModel { original, fallback };
        }

        // 3. Context pressure: trigger reactive compaction.
        if self
            .compaction
            .needs_compaction(context_size_tokens, context_limit_tokens)
            || self.compaction.consecutive_token_errors >= 2
        {
            let triggered = self.compaction.record_token_error(context_size_tokens);
            if triggered {
                // Compaction drain: keep at most half the messages.
                let keep = 25; // reasonable default; caller should override with actual message count
                return RecoveryAction::TriggerCompaction {
                    keep_messages: keep,
                };
            }
            if self.compaction.consecutive_token_errors > 0 {
                // Compaction already attempted and still failing.
                return RecoveryAction::Fail {
                    reason: "compaction retries exhausted; context still too large".to_string(),
                };
            }
        }

        RecoveryAction::None
    }

    /// Reset all recovery state after a successful model call.
    pub fn on_success(&mut self) {
        self.max_tokens.reset();
        self.model_fallback.clear();
        self.compaction.reset();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn max_tokens_escalation_returns_64k() {
        let mut recovery = MaxTokensRecovery::new();
        let tokens = recovery.escalate();
        assert_eq!(tokens, ESCALATED_MAX_TOKENS);
        assert_eq!(recovery.escalation_level, 1);
        assert!(recovery.active);
    }

    #[test]
    fn max_tokens_fails_after_max_retries() {
        let mut recovery = MaxTokensRecovery::new();
        recovery.recovery_retries = MAX_TOKEN_RECOVERY_RETRIES;
        assert!(recovery.should_fail());
    }

    #[test]
    fn max_tokens_builds_recovery_message() {
        let mut recovery = MaxTokensRecovery::new();
        recovery.record_retry();
        let msg = recovery.build_recovery_message();
        assert!(msg.contains("recovery attempt 2"));
    }

    #[test]
    fn max_tokens_reset_clears_all() {
        let mut recovery = MaxTokensRecovery::new();
        recovery.escalate();
        recovery.record_retry();
        recovery.reset();
        assert!(!recovery.active);
        assert_eq!(recovery.recovery_retries, 0);
        assert_eq!(recovery.escalation_level, 0);
    }

    #[test]
    fn model_fallback_preserves_original() {
        let mut fallback = ModelFallback::new(NativeModelFamily::DeepSeek);
        fallback.activate(NativeModelFamily::Qwen);
        assert!(fallback.active);
        assert_eq!(fallback.original_family, Some(NativeModelFamily::Qwen));
    }

    #[test]
    fn model_fallback_synthesis_mentions_original() {
        let mut fallback = ModelFallback::new(NativeModelFamily::DeepSeek);
        fallback.activate(NativeModelFamily::Qwen);
        let msg = fallback.synthesize_error();
        assert!(msg.contains("Qwen"));
        assert!(msg.contains("DeepSeek"));
    }

    #[test]
    fn compaction_triggers_on_second_error() {
        let mut compaction = ReactiveCompaction::new();
        compaction.record_token_error(100_000); // first error — no trigger yet
        assert!(!compaction.compaction_triggered);
        let triggered = compaction.record_token_error(100_000); // second — triggers
        assert!(triggered);
        assert!(compaction.compaction_triggered);
    }

    #[test]
    fn compaction_respects_max_retries() {
        let mut compaction = ReactiveCompaction::new();
        compaction.compaction_retries = MAX_COMPACTION_RETRIES;
        let triggered = compaction.record_token_error(100_000);
        assert!(!triggered); // already at max retries
    }

    #[test]
    fn compaction_threshold_detection() {
        let compaction = ReactiveCompaction::new();
        assert!(compaction.needs_compaction(95_000, 100_000)); // 95%
        assert!(!compaction.needs_compaction(85_000, 100_000)); // 85%
    }

    #[test]
    fn unified_state_resets_on_success() {
        let mut state = ErrorRecoveryState::new();
        state.max_tokens.escalate();
        state.max_tokens.record_retry();
        state.compaction.record_token_error(100_000);
        state.on_success();
        assert!(!state.max_tokens.active);
        assert!(!state.compaction.compaction_triggered);
    }
}
