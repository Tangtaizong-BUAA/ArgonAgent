//! Agent kernel facade types.
//!
//! This module is intentionally behavior-preserving at introduction time. The
//! current native loop still owns execution; these types provide the stable
//! extraction target for the doc39 refactor.

pub mod budget_policy;
pub mod compactor;
pub mod context_spine;
pub mod convergence_enforcer;
pub mod conversation_history;
pub mod evidence_ledger;
pub mod kernel;
pub mod observation_cache;
pub mod permission_gate;
pub mod permission_policy;
pub mod provider_capability;
pub mod telemetry;
pub mod tool_inventory;
pub mod turn_controller;
pub mod turn_router;
pub mod turn_state;
pub mod write_constraints;

pub use budget_policy::BudgetPolicy;
pub use compactor::Compactor;
pub use context_spine::{ContextSpineRef, ContextSpineState, CONTEXT_SPINE_MARKER};
pub use convergence_enforcer::{ConvergenceEnforcer, ConvergenceVerdict};
pub use conversation_history::{
    conversation_messages_from_event_log, conversation_messages_from_tool_result_continuation,
    conversation_messages_to_openai_json, ConversationMessage, ConversationToolCall,
};
pub use evidence_ledger::{
    ContinuationView, EvidenceClass, EvidenceItem, EvidenceLedger, HistoryDigestEntry,
    IterationEvidence, NoveltyScore, SupersededEntry,
};
pub use kernel::{
    AgentKernel, CompletionAuthority, ContextManager, TcmlService, ToolOrchestrationService,
};
pub use observation_cache::{observation_key, DedupeOutcome, ObservationCache};
pub use permission_gate::PermissionGate;
pub use permission_policy::{PermissionDecision, PermissionMode, PermissionPolicy};
pub use provider_capability::{
    capability_cache_file, endpoint_capability_key, provider_mode_label, read_capability_cache,
    write_capability_cache, CapabilityProbe, CapabilityRequirement, ProviderCapabilityIssue,
    ProviderCapabilityMatrix, ProviderToolCallingMode, ToolCallingCapabilities,
    CAPABILITY_CACHE_TTL_SECONDS,
};
pub use telemetry::AgentKernelTelemetry;
pub use tool_inventory::{
    gated_attempt_count as tool_inventory_gated_attempt_count, is_tool_inventory_gated_attempt,
    is_tool_inventory_read_only_observation,
    successful_observation_count as tool_inventory_observation_count, ToolInventoryRecord,
};
pub use turn_controller::{
    estimate_tokens, ContinuationStrategy, IterationOutcome, IterationPreflight,
    LoopConvergenceAction, LoopStopReason, NativeContextGuardAction, NativeContextGuardReport,
    NativeLoopIterationContext, NativeLoopIterationIds, NativeLoopTurnController,
    NativeTurnController, PostToolBatchAction, ToolBatchGuardAction, ToolBatchSignatureStatus,
    ToolIterationControlAction, ToolIterationControlInput, ToolProgressReport, TurnController,
};
pub use turn_router::TurnRouter;
pub use turn_state::{
    AgentRole, AwaitingUserRequest, IterationProgress, ToolProgressDecision, ToolProgressState,
    TurnBudget, TurnConvergenceDecision, TurnConvergenceVerdict, TurnRoute, TurnState,
};
pub use write_constraints::{
    physical_line_count, requested_line_count_policy, validate_file_write_line_count,
    LineCountViolation, RequestedLineCountPolicy,
};
