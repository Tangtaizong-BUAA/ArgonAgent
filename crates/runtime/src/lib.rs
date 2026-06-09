//! Runtime scaffold.

pub mod agent_kernel;
pub mod agent_loop_driver;
pub mod agent_team;
pub mod approval_queue;
pub mod artifact;
pub mod command;
pub mod compaction;
pub mod compatible_provider;
pub mod context_budget;
pub mod context_builder;
pub mod context_policy;
pub mod error_recovery;
pub mod event_invariants;
pub mod event_log;

pub mod executor;
pub mod file_tool;
pub mod git_tool;
pub mod harness;
pub mod hook_dispatcher;
pub mod live_http_transport;
pub mod live_model_executor;
pub mod live_model_request;
pub mod local_api_server;
pub mod model_adapter;
pub mod model_router;
pub mod model_transcript;
pub mod multi_agent_policy;
pub mod native_agent_loop;
pub mod native_profile;
pub mod native_provider;
pub mod native_response_normalizer;
pub mod native_turn_controller;

pub mod parser;
pub mod patch;
pub mod patch_set;
pub mod payload;
pub mod permission_policy;
pub mod prompt_assembler;
pub mod provider_response_adapter;
pub mod qwen_stream;
pub mod recorded_agent_loop;
pub mod recorded_research_loop;
pub mod replay;
pub mod repo_map;
pub mod research_harness;
pub mod research_worker;
pub mod runtime;
pub mod runtime_facade;
pub mod search_tool;
pub mod secret_scan;
pub mod session;
pub mod sidecar_http_transport;
pub mod state;
pub mod subagent;

pub mod tcml;
pub mod tool_dispatcher;
pub mod tool_execution;
pub mod tool_harness;
pub mod tool_orchestration;
pub mod tool_result;
pub mod tool_result_format;

pub mod ultra;
pub mod worktree;

use researchcode_kernel::KernelEvent;

#[derive(Debug, Default)]
pub struct EventBuffer {
    events: Vec<KernelEvent>,
}

impl EventBuffer {
    pub fn push(&mut self, event: KernelEvent) {
        self.events.push(event);
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use researchcode_kernel::Actor;

    #[test]
    fn event_buffer_records_events() {
        let mut buffer = EventBuffer::default();
        buffer.push(KernelEvent {
            event_id: "evt".to_string(),
            schema_version: "v0".to_string(),
            project_id: "proj".to_string(),
            session_id: None,
            task_id: None,
            sequence: 1,
            event_type: "session.created".to_string(),
            actor: Actor::Runtime,
            created_at: "now".to_string(),
            payload_json: "{}".to_string(),
            prev_hash: None,
            hash: "hash".to_string(),
        });
        assert_eq!(buffer.len(), 1);
    }
}
