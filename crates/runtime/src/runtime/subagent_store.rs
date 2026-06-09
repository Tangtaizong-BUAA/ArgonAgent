use crate::session::AgentSession;
use crate::subagent::SubagentSession;
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

#[derive(Debug, Default)]
pub struct SubagentStore {
    subagents: Mutex<HashMap<String, SubagentSession>>,
    sessions: Mutex<HashMap<String, AgentSession>>,
}

impl SubagentStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn subagents_lock(
        &self,
    ) -> Result<
        MutexGuard<'_, HashMap<String, SubagentSession>>,
        PoisonError<MutexGuard<'_, HashMap<String, SubagentSession>>>,
    > {
        self.subagents.lock()
    }

    pub(crate) fn sessions_lock(
        &self,
    ) -> Result<
        MutexGuard<'_, HashMap<String, AgentSession>>,
        PoisonError<MutexGuard<'_, HashMap<String, AgentSession>>>,
    > {
        self.sessions.lock()
    }

    pub(crate) fn insert_subagent(&self, id: String, session: SubagentSession) {
        self.subagents
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, session);
    }

    pub(crate) fn insert_session(&self, id: String, session: AgentSession) {
        self.sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id, session);
    }
}
