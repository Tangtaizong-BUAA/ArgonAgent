# P3/P4 Runtime Completion Status — 2026-05-19

## Verdict

- **P3 AgentKernel authority boundary: implemented and locally verified.**
- **P2-B ConversationHistory/OpenAI JSON: implemented and locally verified.**
- **P4: not complete.** Several P4 prerequisites are present, but the full P4-A test suite and P4-B desktop/runtime UX split are still open.

## P2-B Completed Scope

The cross-turn conversation history path now uses `ConversationHistory`'s OpenAI JSON projection as the runtime-owned history representation:

- `RuntimeFacade::build_context_bundle()` remains project/runtime context only; it does not add `conversation_history.*` context items.
- DeepSeek and Qwen facade live-loop entrypoints now use one shared runtime prompt constructor that injects `# Conversation History (OpenAI JSON)` when prior turn history exists.
- The injected history preserves assistant `tool_calls`, provider `tool_call_id`-bound tool messages, and `reasoning_content` when available.
- The legacy `conversation_messages_to_context_summary` projector was removed, so text history summaries cannot become a competing production path.
- Structured in-turn `tool_result` continuations now derive assistant/tool message layout through `ConversationHistory`'s tool-result continuation projection, then map that projection to DeepSeek OpenAI, DeepSeek Anthropic, or Qwen provider blocks.
- The older direct `*_from_batch` provider-block helpers were removed to avoid keeping `EvidenceLedger` as a competing message-shape owner.

P2-B grep acceptance:

- `conversation_history.summary`: no hits under `crates/runtime/src`.
- `conversation_messages_to_context_summary`: no hits under `crates/runtime/src`.

P2-B focused verification:

- `cargo test -p researchcode-runtime conversation_history -- --test-threads=1`
- `cargo test -p researchcode-runtime facade_context_bundle_excludes_conversation_summary_and_exposes_openai_json -- --test-threads=1`
- `cargo test -p researchcode-runtime facade_injects_openai_conversation_history_tool_call_ids_on_next_turn -- --test-threads=1`
- `cargo test -p researchcode-runtime native_tool_result_continuation_uses_provider_names_and_openai_call_ids -- --test-threads=1`
- `cargo test -p researchcode-runtime native_tool_result_continuation_preserves_provider_openai_call_id -- --test-threads=1`

## P2-C Completed Scope

Transient provider failures now recover on both live transport and native-loop paths:

- `live_http_transport` retries 408 / 409 / 429 / 500 / 502 / 503 / 504 up to 6 attempts and emits `model.http_retry_scheduled` plus `agent.recovery.completed`.
- `native_agent_loop_model_io` now retries the same transient statuses for native-loop streaming and non-streaming sends, replacing the previous `no retry configured` path.
- Retry is skipped for streaming responses after tool-call/visible dirty events have been observed, so already-emitted tool evidence is not replayed unsafely.
- `RuntimeFacade` now owns a request-scoped `ErrorRecoveryState` in the session record and passes a snapshot into DeepSeek/Qwen native loop requests.

P2-C grep acceptance:

- `no retry configured`: no hits in runtime native send paths.
- `error_recovery: None`: no hits in DeepSeek/Qwen facade native-loop request construction.

P2-C focused verification:

- `cargo test -p researchcode-runtime scripted_transport_retries_transient_http_failure_and_records_recovery -- --test-threads=1`
- `cargo test -p researchcode-runtime facade_native_loop_retries_transient_http_failure -- --test-threads=1`

## P3 Completed Scope

The P3 acceptance target was authority ownership, not deleting the native loop. Current code now routes the major decisions through AgentKernel-owned services:

| Decision class | Current authority |
|---|---|
| Context guard / prepared request guard | `AgentKernel.context_manager` |
| Permission decisions for native loop tool paths | request-scoped `AgentKernel.permission_gate` |
| TCML text parse / final answer extraction | `AgentKernel.tcml` |
| Repeated batch / convergence guard | `AgentKernel.tool_orchestration` plus `NativeLoopTurnController` |
| Finalizer / fallback can-finalize policy | `AgentKernel.finalizer` |
| Tool partition / schedule dispatch | `AgentKernel.tool_orchestration` |

Notable code points:

- `crates/runtime/src/agent_kernel/kernel.rs`
- `crates/runtime/src/native_agent_loop.rs`
- `crates/runtime/src/native_agent_loop_tools.rs`
- `crates/runtime/src/native_agent_loop_execution.rs`
- `crates/runtime/src/native_agent_loop_stream.rs`
- `crates/runtime/src/native_agent_loop_completion.rs`

## P3 Grep Acceptance

Current grep results after the P3 pass:

- `guard_native_model_request(`: no remaining call-sites.
- `evaluate_permission_request(`: no native-loop production call-sites; one `RuntimeFacade` compatibility/manual tool-mode path remains.
- `partition_tool_calls(` / `schedule_tool_calls(` / `observe_tool_batch_guard(`: native-loop production paths route through `AgentKernel.tool_orchestration`; direct calls remain only inside the kernel service wrapper.
- `NativeLoopTurnController::new()` / `EvidenceLedger::default()` / `ConvergenceEnforcer::default()`: no native-loop production construction; construction remains in `AgentKernel::default()` and tests.
- CLI product path uses `RuntimeFacade`; `RuntimeFacade` still contains an explicit legacy compatibility boundary for V1 and interrupt-aware V2 entrypoints.

## Verification Run

Passed:

- `cargo test -p researchcode-runtime kernel_ -- --test-threads=1`
- `cargo test -p researchcode-runtime native_agent_loop_v2_rebuilds_request_after_preflight_compaction -- --test-threads=1`
- `cargo test -p researchcode-runtime native_loop_fastauto_write_helper_blocks_sensitive_path_without_outer_gate -- --test-threads=1`
- `cargo test -p researchcode-runtime native_agent_loop_v2_recovers_from_repeated_tool_batch -- --test-threads=1`
- `cargo test -p researchcode-runtime visible_finalizer_schema_exposes_only_agent_final_answer -- --test-threads=1`
- `cargo check -p researchcode-runtime`
- `cargo check -p researchcode-cli`
- `npm run build` in `desktop`
- `cargo test -p researchcode-runtime --lib -- --test-threads=1` outside sandbox: `610 passed`

Sandbox note: the full runtime lib suite fails inside sandbox only on local API server bind permissions (`127.0.0.1:0 Operation not permitted`); outside sandbox it passes.

## P4-A Status: Test Hardening

Current status: **partial**.

Implemented or present:

- HTTP retry tests for transient failures exist in `live_http_transport`.
- StreamProcessor unit tests exist under `native_profile/deepseek/stream_processor.rs`.
- A facade/native-loop session reopen test now covers 3 sequential turns and forces a facade context compaction boundary before turn 3:
  - `facade_deepseek_loop_reopens_completed_session_for_next_turn`
- A native-loop concurrent read-only batch regression test was added:
  - `native_agent_loop_v2_concurrent_read_only_batch_preserves_evidence_ordering`

Still open:

- A larger fixed replay fixture around 193K token projected prompts with explicit reasoning preservation assertions.
- Real subagent execution test covering Explorer -> Worker handoff, event merge, and isolation failure.
- `live-tests` gated smoke test for real API environments.
- Fixed compaction replay fixture around a large projected prompt threshold.
- More explicit 429 -> 200 and 503 -> 503 -> 200 native-loop/provider-adapter coverage if required above transport-level tests.

## P4-B Status: Desktop UX / Performance

Current status: **partial**.

Implemented or present:

- Tauri push subscription exists.
- The 400ms polling loop is skipped when Tauri push is active.
- Transcript uses memoized components for message and tool activity rendering.
- Runtime `model.stream_delta` payloads now carry `runtime_sanitized: true`; the GUI uses that marker to avoid the expensive full tool-markup sanitizer on the normal runtime-authored stream path.

Still open:

- `sanitizeStreamChunk` still lives in `desktop/src/components/AppShell.tsx` as a compatibility fallback for older/unmarked events.
- Transcript virtualization/windowing is not implemented; memoization helps but does not solve very long transcript render cost.
- There is no dedicated desktop performance regression harness for long event streams.

## P4 StreamProcessor Status From The Older Upgrade Plan

The older `upgrade_plan_2026_05_18.md` names Phase 4 as "StreamProcessor 状态机化". That item is **mostly implemented**:

- `crates/runtime/src/native_profile/deepseek/stream_processor.rs` exists.
- Runtime/provider paths instantiate `StreamProcessor`.
- Tests cover DSML filtering, tool assembly, visible/reasoning deltas, and suppression.

Remaining nuance:

- Some lower-level `DsmlChunkFilter` and `StreamingToolCallAssembler` tests remain by design.
- Desktop still has a defensive stream sanitizer, so sanitation authority is not visually/auditorially single-sourced all the way to the GUI.

## Next Recommended P4 Slice

1. Add a true 3+ turn scripted e2e that crosses compaction and verifies prompt projection size plus reasoning/evidence replay.
2. Add subagent execution regression covering event merge and isolation denial.
3. Move GUI stream sanitization to a runtime-authored normalized event contract, then make the frontend sanitizer a no-op fallback with tests.
4. Add transcript virtualization or chunked rendering for long sessions.
