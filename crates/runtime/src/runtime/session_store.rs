use crate::error_recovery::ErrorRecoveryState;
use crate::permission_policy::PermissionRuleSet;
use crate::runtime_facade::{RuntimePendingNativeDecision, RuntimeSessionHandle};
use crate::session::AgentSession;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

#[derive(Debug)]
pub(crate) struct RuntimeSessionRecord {
    pub(crate) handle: RuntimeSessionHandle,
    pub(crate) session: AgentSession,
    pub(crate) session_policy: PermissionRuleSet,
    pub(crate) session_memory: Vec<String>,
    pub(crate) file_state: HashMap<String, RuntimeFileState>,
    pub(crate) plan_mode_active: bool,
    pub(crate) repeated_tool_failures: HashMap<String, usize>,
    pub(crate) path_corrections: HashMap<String, String>,
    pub(crate) discovered_roots: Vec<String>,
    pub(crate) native_tool_completion: HashMap<String, bool>,
    pub(crate) error_recovery: ErrorRecoveryState,
    pub(crate) pending_native_decision: Option<RuntimePendingNativeDecision>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeFileState {
    pub(crate) path: String,
    pub(crate) content_hash: String,
    pub(crate) line_start: Option<u64>,
    pub(crate) line_end: Option<u64>,
    pub(crate) read_ranges: Vec<(u64, u64)>,
}

#[derive(Debug, Default)]
pub struct SessionStore {
    sessions: Mutex<HashMap<String, RuntimeSessionRecord>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn insert(&self, id: String, record: RuntimeSessionRecord) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, record);
    }

    pub(crate) fn lock(
        &self,
    ) -> Result<
        MutexGuard<'_, HashMap<String, RuntimeSessionRecord>>,
        PoisonError<MutexGuard<'_, HashMap<String, RuntimeSessionRecord>>>,
    > {
        self.sessions.lock()
    }

    pub(crate) fn remove(&self, id: &str) -> Option<RuntimeSessionRecord> {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id)
    }

    pub(crate) fn with_ref<R>(
        &self,
        id: &str,
        f: impl FnOnce(&RuntimeSessionRecord) -> R,
    ) -> Option<R> {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.get(id).map(f)
    }

    pub(crate) fn with_mut<R>(
        &self,
        id: &str,
        f: impl FnOnce(&mut RuntimeSessionRecord) -> R,
    ) -> Option<R> {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.get_mut(id).map(f)
    }

    #[allow(dead_code)]
    pub(crate) fn with_all<R>(
        &self,
        f: impl FnOnce(&HashMap<String, RuntimeSessionRecord>) -> R,
    ) -> R {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&sessions)
    }

    #[allow(dead_code)]
    pub(crate) fn with_all_mut<R>(
        &self,
        f: impl FnOnce(&mut HashMap<String, RuntimeSessionRecord>) -> R,
    ) -> R {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut sessions)
    }
}
