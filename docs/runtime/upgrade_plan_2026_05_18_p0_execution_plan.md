# Upgrade Plan 2026-05-18 P0 Execution Plan

Source contract:

- `/Users/gongyuxuan/Documents/deep-code/upgrade_plan_2026_05_18.md`

Scope:

- `crates/runtime/src/`
- focused tests/fixtures needed to prove the P0 runtime behavior
- documentation updates that record the implemented evidence

Denied in this slice:

- full TCML migration
- full TurnController extraction
- async runtime dependency additions
- provider-layer redesign
- destructive git operations

## Current-Code Cross-Check

P0-1 is repaired. Permission policy denial now becomes a structured
model-readable tool result instead of an `Err` that aborts the turn. The runtime
also records a `tool.model_readable_error` event so GUI/event consumers can show
the denial without opening the artifact.

P0-2 is repaired in the current code. The native loop records progress through
`TurnState::record_tool_iteration_from_observation_cache`, so the no-progress
decision is based on `ObservationCache` distinct-key growth rather than any OK
non-duplicate result.

P0-3 is repaired. `NativeAgentLoopRequest`, `NativeAgentLoopV2Request`, and
resume request paths carry the five-mode `PermissionMode` plus provided
decisions. Write/shell/patch paths use the effective request mode instead of
silently downgrading to `Default`.

P0-4 is repaired and locked by tests. Streaming `file.write` / `file.edit` /
`file.multi_edit` and patch/write permission paths run through
`PermissionResolver` / dangerous path checks before execution, including
`BypassPermissions` fast-auto attempts.

P0-5 is repaired. `send_with_live_visible_stream_events` buffers visible and
thinking stream deltas until a 2xx response is accepted; deltas from a failed
400 first attempt are discarded when dual-protocol fallback retries the request.

P0-6 is repaired. Plain-evidence continuation now includes both the provider
`tool_call_id` and runtime `result_tool_call_id`, preserving binding cues even
when the provider cannot accept structured `tool_result` replay.

## Implementation Tasks

1. Permission-deny tool result: completed.
   - replace deny `Err` with a structured `PermissionDenied` model-readable tool
     result for collect paths;
   - preserve pending permission behavior for `Ask`;
   - add tests for denied shell/write paths proving the loop continues instead
     of failing the turn.

2. Progress plateau: completed/verified against current code.
   - record per-iteration progress into `turn_state.progress`;
   - classify distinct-key growth, duplicate suppression, recovery, and errors;
   - on `ToolProgressDecision::Finalize`, finalize through the existing visible
     fallback/finalizer path;
   - add a repeated no-progress fixture.

3. Permission modes: completed.
   - verify no `NativeAgentPermissionMode` remains;
   - ensure native loop request `permission_mode` is used on classic command and
     patch paths;
   - ensure FastAuto/CodeEdit v2 write branches also route through effective
     request permission mode before execution.

4. Dangerous paths: completed.
   - add focused streaming/non-streaming fixtures for `.env` and `.ssh/id_rsa`
     with `file.write` and `file.edit`;
   - ensure dangerous paths produce permission/safety feedback and never write
     directly in bypass/fast-auto paths.

5. Streaming fallback events: completed.
   - buffer live visible stream deltas inside `send_with_live_visible_stream_events`;
   - flush only when the returned response is the accepted response;
   - do not flush first-attempt deltas if a dual-protocol fallback replaces the
     request;
   - add a 400-then-success fixture proving only fallback-success deltas are
     emitted.

6. Plain evidence markers: completed.
   - include `tool_call_id`, `provider_tool_call_id` when known, and `tool_id`
     in the plain text continuation evidence;
   - add a test for `build_native_tool_evidence_continuation_request`.

## Verification

Focused checks:

- `cargo test -p researchcode-runtime --lib native_permission -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_permission_denial_returns_tool_result -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_streaming_write_to_sensitive_path_requests_permission -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib live_visible_stream -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_tool_evidence_continuation -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib turn_state_progress -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_empty_visible -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_deepseek_anthropic_continues_with_plain_evidence_after_tools -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_suppresses_stream_preamble_when_tool_call_starts -- --test-threads=1`

Final checks:

- `cargo fmt`
- `cargo test -p researchcode-runtime --lib -- --test-threads=1`
- `cargo test -p researchcode-kernel --lib -- --test-threads=1`
