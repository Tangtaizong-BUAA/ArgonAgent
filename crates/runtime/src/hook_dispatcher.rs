//! Sync bounded hook dispatcher.

use researchcode_kernel::hooks::{
    Hook, HookDecision, HookDispatchPolicy, HookEvent, HookTimeoutPolicy,
};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

/// Maximum number of concurrent hook executions.
const MAX_CONCURRENT_HOOKS: usize = 8;

/// A simple semaphore for bounding concurrent hook threads.
struct ConcurrencyLimiter {
    inner: Arc<(Mutex<usize>, Condvar)>,
}

impl ConcurrencyLimiter {
    fn new(max_permits: usize) -> Self {
        Self {
            inner: Arc::new((Mutex::new(max_permits), Condvar::new())),
        }
    }

    fn acquire(&self) -> ConcurrencyGuard {
        let (ref lock, ref cvar) = *self.inner;
        let mut permits = lock.lock().unwrap();
        while *permits == 0 {
            permits = cvar.wait(permits).unwrap();
        }
        *permits -= 1;
        ConcurrencyGuard {
            inner: self.inner.clone(),
        }
    }
}

struct ConcurrencyGuard {
    inner: Arc<(Mutex<usize>, Condvar)>,
}

impl Drop for ConcurrencyGuard {
    fn drop(&mut self) {
        let (ref lock, ref cvar) = *self.inner;
        let mut permits = lock.lock().unwrap();
        *permits += 1;
        cvar.notify_one();
    }
}

impl Clone for ConcurrencyLimiter {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HookDispatchStatus {
    Skipped,
    Completed,
    Timeout,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookDispatchOutcome {
    pub hook_index: usize,
    pub status: HookDispatchStatus,
    pub decision: HookDecision,
    pub warning: Option<String>,
}

#[derive(Clone)]
pub struct HookDispatcher {
    hooks: Vec<Arc<dyn Hook>>,
    policy: HookDispatchPolicy,
    concurrency_limiter: ConcurrencyLimiter,
}

impl std::fmt::Debug for HookDispatcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookDispatcher")
            .field("hook_count", &self.hooks.len())
            .field("policy", &self.policy)
            .finish()
    }
}

impl PartialEq for HookDispatcher {
    fn eq(&self, other: &Self) -> bool {
        self.hooks.len() == other.hooks.len() && self.policy == other.policy
    }
}

impl Eq for HookDispatcher {}

impl HookDispatcher {
    pub fn new(policy: HookDispatchPolicy) -> Self {
        Self {
            hooks: Vec::new(),
            policy,
            concurrency_limiter: ConcurrencyLimiter::new(MAX_CONCURRENT_HOOKS),
        }
    }

    pub fn register(&mut self, hook: Arc<dyn Hook>) {
        self.hooks.push(hook);
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    pub fn dispatch(&self, event: &HookEvent) -> Vec<HookDispatchOutcome> {
        self.hooks
            .iter()
            .enumerate()
            .filter_map(|(hook_index, hook)| {
                if !hook.matches(event) {
                    return None;
                }
                Some(self.dispatch_one(hook_index, hook.clone(), event.clone()))
            })
            .collect()
    }

    fn dispatch_one(
        &self,
        hook_index: usize,
        hook: Arc<dyn Hook>,
        event: HookEvent,
    ) -> HookDispatchOutcome {
        let timeout = hook_timeout(&*hook, &self.policy);
        let (sender, receiver) = mpsc::channel();
        // Acquire a concurrency slot before spawning.
        // NOTE: If a hook thread is abandoned after a timeout, the guard is leaked
        // on purpose — the spawned thread is detached and its slot will not be
        // returned. In practice this limits total slots to MAX_CONCURRENT_HOOKS ×
        // number of timeouts. A future improvement could use a timeout-aware pool.
        let guard = self.concurrency_limiter.acquire();
        thread::spawn(move || {
            let _slot = guard; // return slot when this thread exits or panics
            let decision = hook.handle(&event);
            let _ = sender.send(decision);
        });
        match receiver.recv_timeout(timeout) {
            Ok(decision) => HookDispatchOutcome {
                hook_index,
                status: HookDispatchStatus::Completed,
                decision,
                warning: None,
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let decision = match self.policy.timeout_policy {
                    HookTimeoutPolicy::DenyWithError => HookDecision::Deny {
                        reason: "hook timed out".to_string(),
                    },
                    HookTimeoutPolicy::AllowWithWarning => HookDecision::Warn {
                        warning: "hook timed out; allowing with warning".to_string(),
                    },
                };
                HookDispatchOutcome {
                    hook_index,
                    status: HookDispatchStatus::Timeout,
                    decision,
                    warning: Some("hook timed out".to_string()),
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => HookDispatchOutcome {
                hook_index,
                status: HookDispatchStatus::Failed,
                decision: HookDecision::Warn {
                    warning: "hook failed before returning a decision".to_string(),
                },
                warning: Some("hook failed before returning a decision".to_string()),
            },
        }
    }
}

fn hook_timeout(hook: &dyn Hook, policy: &HookDispatchPolicy) -> Duration {
    let timeout = hook.timeout();
    if timeout.is_zero() {
        Duration::from_millis(policy.default_timeout_ms)
    } else {
        timeout
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use researchcode_kernel::hooks::HookEvent;

    struct DenyHook;

    impl Hook for DenyHook {
        fn matches(&self, event: &HookEvent) -> bool {
            matches!(event, HookEvent::PreToolUse { tool_id, .. } if tool_id == "file.write")
        }

        fn handle(&self, _event: &HookEvent) -> HookDecision {
            HookDecision::Deny {
                reason: "write blocked by test hook".to_string(),
            }
        }
    }

    struct SlowHook;

    impl Hook for SlowHook {
        fn matches(&self, _event: &HookEvent) -> bool {
            true
        }

        fn handle(&self, _event: &HookEvent) -> HookDecision {
            thread::sleep(Duration::from_millis(50));
            HookDecision::Allow
        }

        fn timeout(&self) -> Duration {
            Duration::from_millis(1)
        }
    }

    #[test]
    fn dispatcher_collects_matching_hook_decisions() {
        let mut dispatcher = HookDispatcher::new(HookDispatchPolicy::default());
        dispatcher.register(Arc::new(DenyHook));
        let outcomes = dispatcher.dispatch(&HookEvent::PreToolUse {
            tool_id: "file.write".to_string(),
            args_json: "{\"path\":\"demo.txt\"}".to_string(),
            provider_tool_use_id: Some("toolu_write".to_string()),
        });

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, HookDispatchStatus::Completed);
        assert_eq!(
            outcomes[0].decision,
            HookDecision::Deny {
                reason: "write blocked by test hook".to_string()
            }
        );
    }

    #[test]
    fn dispatcher_timeout_defaults_to_deny() {
        let mut dispatcher = HookDispatcher::new(HookDispatchPolicy::default());
        dispatcher.register(Arc::new(SlowHook));
        let outcomes = dispatcher.dispatch(&HookEvent::UserPromptSubmit {
            text: "hello".to_string(),
            attachments: Vec::new(),
        });

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, HookDispatchStatus::Timeout);
        assert!(matches!(outcomes[0].decision, HookDecision::Deny { .. }));
    }

    #[test]
    fn dispatcher_timeout_with_deny_policy_returns_deny() {
        let policy = HookDispatchPolicy {
            timeout_policy: HookTimeoutPolicy::DenyWithError,
            ..HookDispatchPolicy::default()
        };
        let mut dispatcher = HookDispatcher::new(policy);
        dispatcher.register(Arc::new(SlowHook));
        let outcomes = dispatcher.dispatch(&HookEvent::UserPromptSubmit {
            text: "hello".to_string(),
            attachments: Vec::new(),
        });

        assert_eq!(outcomes.len(), 1);
        assert_eq!(outcomes[0].status, HookDispatchStatus::Timeout);
        assert!(matches!(outcomes[0].decision, HookDecision::Deny { .. }));
    }
}
