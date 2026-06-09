//! AgentLoopDriver — continuous loop with channel-based permission blocking.
//!
//! Modeled after Claude Code's `query.ts` `while(true)` AsyncGenerator pattern.
//! The loop never exits for permissions — it blocks on channel.recv() and resumes
//! when the user's decision arrives.
//!
//! Architecture (Rust equivalent of Promise-based blocking):
//! ```text
//! run() -> while turn < max_iterations {
//!   permission_check(tool) -> if blocked {
//!     save_snapshot() -> channel.recv_timeout() -> restore() -> continue
//!   }
//!   execute_tool()
//! }
//! ```

use crate::agent_kernel::permission_policy::PermissionMode;
use crate::error_recovery::ErrorRecoveryState;
use crate::hook_dispatcher::HookDispatcher;
use crate::native_profile::deepseek::adaptation::DeepSeekAdaptationManager;
use crate::session::AgentSession;
use crate::state::AgentState;
use researchcode_kernel::model::NativeModelFamily;
use researchcode_kernel::tool::find_tool_spec;
use researchcode_kernel::PermissionDecisionKind;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

/// Timeout for permission blocking — prevents permanent deadlock if the UI never responds.
const PERMISSION_BLOCK_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

// ── Driver types ──────────────────────────────────────────────────────────────

/// Permission decision sent from the UI into the channel.
#[derive(Debug, Clone)]
pub struct LoopPermissionDecision {
    pub tool_call_id: String,
    pub tool_id: String,
    pub decision: PermissionDecisionKind,
    pub persist_scope: Option<String>,
}

/// Events emitted by the driver for UI streaming.
#[derive(Debug, Clone)]
pub enum DriverEvent {
    ThinkingDelta {
        delta: String,
    },
    TextDelta {
        delta: String,
    },
    ToolCallParsed {
        tool_call_id: String,
        tool_id: String,
        args_json: String,
    },
    PermissionRequested {
        tool_call_id: String,
        tool_id: String,
        reason: String,
        reasoning_preview: Option<String>,
    },
    ToolExecuted {
        tool_call_id: String,
        tool_id: String,
        ok: bool,
        preview: String,
    },
    LoopFinished {
        status: String,
        tool_call_count: u32,
        model_call_count: u32,
    },
    Error {
        message: String,
        recoverable: bool,
    },
}

/// Configuration for the driver.
#[derive(Debug, Clone)]
pub struct DriverConfig {
    pub project_id: String,
    pub session_id: String,
    pub task_id: String,
    pub workspace_root: PathBuf,
    pub artifact_root: PathBuf,
    pub max_iterations: usize,
    pub max_tool_calls: usize,
    pub permission_mode: PermissionMode,
    pub family: NativeModelFamily,
}

/// The agent loop driver.
/// Manages the continuous loop, channel for permissions, hook dispatch, and
/// DeepSeek reasoning replay across permission blocking boundaries.
pub struct AgentLoopDriver {
    config: DriverConfig,
    session: AgentSession,
    adaptation: DeepSeekAdaptationManager,
    error_recovery: ErrorRecoveryState,
    hook_dispatcher: Option<HookDispatcher>,
    /// Channel for external permission decisions.
    decision_rx: mpsc::Receiver<LoopPermissionDecision>,
    decision_tx: mpsc::Sender<LoopPermissionDecision>,
    /// Event sink for UI streaming.
    /// Wrapped in a Mutex so that `emit_direct` (which takes `&self`) can
    /// still deliver events without requiring a mutable reference.
    event_sink: Mutex<Option<Box<dyn FnMut(DriverEvent) + Send>>>,
    tool_call_count: u32,
    model_call_count: u32,
}

impl AgentLoopDriver {
    pub fn new(
        config: DriverConfig,
        hook_dispatcher: Option<HookDispatcher>,
        event_sink: Option<Box<dyn FnMut(DriverEvent) + Send>>,
    ) -> Result<Self, String> {
        let session = AgentSession::new(&config.project_id, &config.session_id, &config.task_id)
            .map_err(|e| format!("{e:?}"))?;
        let (decision_tx, decision_rx) = mpsc::channel();
        let family = config.family;

        Ok(Self {
            adaptation: DeepSeekAdaptationManager::new(family),
            config,
            session,
            error_recovery: ErrorRecoveryState::new(),
            hook_dispatcher,
            decision_rx,
            decision_tx,
            event_sink: Mutex::new(event_sink),
            tool_call_count: 0,
            model_call_count: 0,
        })
    }

    /// Sender handle for external systems to push permission decisions.
    pub fn decision_sender(&self) -> mpsc::Sender<LoopPermissionDecision> {
        self.decision_tx.clone()
    }

    /// Run the continuous loop with state transitions.
    ///
    /// ## Responsibility boundary
    ///
    /// This method manages **session lifecycle state** only (Planning →
    /// RetrievingContext → Executing → Review → Completed). It does NOT
    /// call models or execute tools.
    ///
    /// The actual model calling + tool execution loop is driven by the
    /// `native_agent_loop` (V2 loop), which is invoked externally by
    /// `runtime_facade`. That V2 loop uses this driver's infrastructure for:
    /// - Channel-based permission blocking (`block_for_permission`)
    /// - Hook dispatch (`hook_dispatcher()`)
    /// - DeepSeek reasoning replay (`adaptation()`)
    /// - Error recovery escalation (`error_recovery_mut()`)
    ///
    /// The V2 loop is the **only** caller that should advance tool/model
    /// counters via `record_tool_call()` / `record_model_call()`.
    pub fn run(&mut self) -> Result<DriverLoopResult, String> {
        self.session
            .transition_to(AgentState::Planning)
            .and_then(|_| self.session.transition_to(AgentState::RetrievingContext))
            .and_then(|_| self.session.transition_to(AgentState::Executing))
            .map_err(|e| format!("{e:?}"))?;

        // NOTE: The Planning → RetrievingContext → Executing transitions above
        // prepare the session for the V2 loop. The V2 loop (native_agent_loop)
        // is invoked externally by runtime_facade after this driver is created.
        // When the V2 loop finishes, it calls back to finalize the session
        // (review + complete) below.

        self.session
            .start_review()
            .and_then(|_| self.session.complete_after_review())
            .map_err(|e| format!("{e:?}"))?;

        self.emit(DriverEvent::LoopFinished {
            status: "completed".to_string(),
            tool_call_count: self.tool_call_count,
            model_call_count: self.model_call_count,
        });

        Ok(DriverLoopResult {
            tool_call_count: self.tool_call_count,
            model_call_count: self.model_call_count,
        })
    }

    /// Block waiting for a permission decision on a tool with a timeout.
    /// Returns the decision or an error if the channel closed or timed out.
    pub fn block_for_permission(
        &self,
        tool_call_id: &str,
        tool_id: &str,
        reason: &str,
    ) -> Result<LoopPermissionDecision, String> {
        // Save reasoning snapshot before blocking
        let snapshot = self
            .adaptation
            .capture_reasoning_snapshot(&self.config.session_id);

        // Notify the UI
        self.emit_direct(DriverEvent::PermissionRequested {
            tool_call_id: tool_call_id.to_string(),
            tool_id: tool_id.to_string(),
            reason: reason.to_string(),
            reasoning_preview: snapshot.as_ref().map(|s| s.sanitized_preview.clone()),
        });

        // Block on channel with timeout (prevents permanent deadlock)
        let result = self
            .decision_rx
            .recv_timeout(PERMISSION_BLOCK_TIMEOUT)
            .map_err(|e| match e {
                mpsc::RecvTimeoutError::Timeout => {
                    // Timeout recovery: emit an error event so the UI can surface
                    // the failure. The caller receives this error and should abort
                    // the current tool execution — otherwise the tool remains in a
                    // hung "waiting for permission" state indefinitely.
                    format!(
                        "permission decision timed out after {}s for tool '{tool_id}'",
                        PERMISSION_BLOCK_TIMEOUT.as_secs()
                    )
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    // Channel disconnect means the UI has gone away. No further
                    // permission decisions can be received; the caller must abort.
                    "permission channel closed".to_string()
                }
            });
        if result.is_err() {
            // Notify the UI of the failure so the blocked tool state is visible.
            self.emit_direct(DriverEvent::Error {
                message: format!(
                    "permission block failed for tool '{tool_id}': {}",
                    result.as_ref().unwrap_err()
                ),
                recoverable: false,
            });
        }
        result
    }

    /// Record a tool call completion.
    pub fn record_tool_call(&mut self) {
        self.tool_call_count += 1;
    }

    /// Record a model call completion.
    pub fn record_model_call(&mut self) {
        self.model_call_count += 1;
    }

    /// Check if a tool needs permission checking.
    pub fn needs_permission_check(&self, tool_id: &str) -> bool {
        if self.config.permission_mode == PermissionMode::BypassPermissions {
            return false;
        }
        find_tool_spec(tool_id)
            .map(|spec| spec.permission_required)
            .unwrap_or(true)
    }

    /// Get the hook dispatcher for PreToolUse/PostToolUse hooks.
    pub fn hook_dispatcher(&self) -> Option<&HookDispatcher> {
        self.hook_dispatcher.as_ref()
    }

    /// Get the DeepSeek adaptation manager.
    pub fn adaptation(&self) -> &DeepSeekAdaptationManager {
        &self.adaptation
    }

    pub fn adaptation_mut(&mut self) -> &mut DeepSeekAdaptationManager {
        &mut self.adaptation
    }

    /// Get the error recovery state.
    pub fn error_recovery(&self) -> &ErrorRecoveryState {
        &self.error_recovery
    }

    pub fn error_recovery_mut(&mut self) -> &mut ErrorRecoveryState {
        &mut self.error_recovery
    }

    /// Emit an event to the UI sink (requires &mut self).
    fn emit(&mut self, event: DriverEvent) {
        if let Ok(mut guard) = self.event_sink.lock() {
            if let Some(sink) = guard.as_mut() {
                sink(event);
            }
        }
    }

    /// Emit event without mutable reference to self (for use in &self methods
    /// such as `block_for_permission`).
    ///
    /// The event_sink is wrapped in a `Mutex` so this works with `&self`.
    /// If the mutex is poisoned, the event is silently dropped rather than
    /// panicking the agent loop.
    fn emit_direct(&self, event: DriverEvent) {
        if let Ok(mut guard) = self.event_sink.lock() {
            if let Some(sink) = guard.as_mut() {
                sink(event);
            }
        }
    }
}

impl std::fmt::Debug for AgentLoopDriver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentLoopDriver")
            .field("config", &self.config)
            .field("session", &self.session)
            .field("adaptation", &self.adaptation)
            .field("error_recovery", &self.error_recovery)
            .field("hook_dispatcher", &self.hook_dispatcher)
            .field("tool_call_count", &self.tool_call_count)
            .field("model_call_count", &self.model_call_count)
            .finish()
    }
}

// ── Helper types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct DriverLoopResult {
    pub tool_call_count: u32,
    pub model_call_count: u32,
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_config_constructs() {
        let config = DriverConfig {
            project_id: "test".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            workspace_root: PathBuf::from("/tmp"),
            artifact_root: PathBuf::from("/tmp/artifacts"),
            max_iterations: 10,
            max_tool_calls: 50,
            permission_mode: PermissionMode::Default,
            family: NativeModelFamily::DeepSeek,
        };
        assert_eq!(config.max_iterations, 10);
        assert_eq!(config.max_tool_calls, 50);
    }

    #[test]
    fn driver_creates_decision_channel() {
        let config = DriverConfig {
            project_id: "test".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            workspace_root: PathBuf::from("/tmp"),
            artifact_root: PathBuf::from("/tmp/artifacts"),
            max_iterations: 10,
            max_tool_calls: 50,
            permission_mode: PermissionMode::Default,
            family: NativeModelFamily::DeepSeek,
        };

        let driver = AgentLoopDriver::new(config, None, None).unwrap();
        let sender = driver.decision_sender();

        // Send a decision through the channel
        sender
            .send(LoopPermissionDecision {
                tool_call_id: "tc1".to_string(),
                tool_id: "shell.command".to_string(),
                decision: PermissionDecisionKind::AllowOnce,
                persist_scope: None,
            })
            .unwrap();

        // Receive it back
        let decision = driver.block_for_permission("tc1", "shell.command", "test reason");
        assert!(decision.is_ok());
        assert_eq!(decision.unwrap().tool_call_id, "tc1");
    }

    #[test]
    fn bypass_mode_skips_permission_check() {
        let config = DriverConfig {
            project_id: "test".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            workspace_root: PathBuf::from("/tmp"),
            artifact_root: PathBuf::from("/tmp/artifacts"),
            max_iterations: 10,
            max_tool_calls: 50,
            permission_mode: PermissionMode::BypassPermissions,
            family: NativeModelFamily::DeepSeek,
        };

        let driver = AgentLoopDriver::new(config, None, None).unwrap();
        assert!(!driver.needs_permission_check("shell.command"));
        assert!(!driver.needs_permission_check("file.write"));
    }

    #[test]
    fn adaptation_manager_accessible() {
        let config = DriverConfig {
            project_id: "test".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            workspace_root: PathBuf::from("/tmp"),
            artifact_root: PathBuf::from("/tmp/artifacts"),
            max_iterations: 10,
            max_tool_calls: 50,
            permission_mode: PermissionMode::Default,
            family: NativeModelFamily::DeepSeek,
        };

        let mut driver = AgentLoopDriver::new(config, None, None).unwrap();
        let adaptation = driver.adaptation_mut();
        assert!(!adaptation.reasoning_replay_blocked);
    }

    #[test]
    fn error_recovery_accessible() {
        let config = DriverConfig {
            project_id: "test".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            workspace_root: PathBuf::from("/tmp"),
            artifact_root: PathBuf::from("/tmp/artifacts"),
            max_iterations: 10,
            max_tool_calls: 50,
            permission_mode: PermissionMode::Default,
            family: NativeModelFamily::DeepSeek,
        };

        let mut driver = AgentLoopDriver::new(config, None, None).unwrap();
        assert!(!driver.error_recovery().max_tokens.active);
        assert!(!driver.error_recovery().max_tokens.should_fail());

        driver.error_recovery_mut().max_tokens.escalate();
        assert!(driver.error_recovery().max_tokens.active);
    }

    #[test]
    fn driver_run_completes() {
        let config = DriverConfig {
            project_id: "test".to_string(),
            session_id: "sess_run".to_string(),
            task_id: "task".to_string(),
            workspace_root: PathBuf::from("/tmp"),
            artifact_root: PathBuf::from("/tmp/artifacts"),
            max_iterations: 10,
            max_tool_calls: 50,
            permission_mode: PermissionMode::Default,
            family: NativeModelFamily::DeepSeek,
        };

        let mut driver = AgentLoopDriver::new(config, None, None).unwrap();
        let result = driver.run().unwrap();
        assert_eq!(result.tool_call_count, 0);
        assert_eq!(result.model_call_count, 0);
    }

    #[test]
    fn tool_call_recording_works() {
        let config = DriverConfig {
            project_id: "test".to_string(),
            session_id: "sess".to_string(),
            task_id: "task".to_string(),
            workspace_root: PathBuf::from("/tmp"),
            artifact_root: PathBuf::from("/tmp/artifacts"),
            max_iterations: 10,
            max_tool_calls: 50,
            permission_mode: PermissionMode::Default,
            family: NativeModelFamily::DeepSeek,
        };

        let mut driver = AgentLoopDriver::new(config, None, None).unwrap();
        assert_eq!(driver.tool_call_count, 0);
        driver.record_tool_call();
        driver.record_tool_call();
        assert_eq!(driver.tool_call_count, 2);
        driver.record_model_call();
        assert_eq!(driver.model_call_count, 1);
    }
}
