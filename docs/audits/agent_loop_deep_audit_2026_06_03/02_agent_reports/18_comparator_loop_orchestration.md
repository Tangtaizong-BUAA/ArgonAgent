Now I have all the data I need. Let me compile the architectural review.

---

## Architecture Review: deep-code (Rust) vs OpenClaudeCode (TypeScript)
### Agent Loop Event/Turn Orchestration

---

### 1. Control Flow: CRITICAL divergence

**OCC** (`query.ts` lines 307-1728): Single `while(true)` loop. Each iteration is one clean cycle: compact -> call model -> collect response -> execute tools -> build next state -> continue. All iterations follow the same path regardless of whether it is the first call or a follow-up. Continuation is just `messages: [...messagesForQuery, ...assistantMessages, ...toolResults]`.

**deep-code** (`native_agent_loop.rs` lines 556-2879): `for iteration in 0..max_iterations` with a massive `if !last_tool_batch.is_empty()` / `else` split (line 627). The "initial request" path (lines 1080-1401, ~320 lines) and "continuation" path (lines 627-1079, ~450 lines) are entirely separate code blocks with their own tool execution, guard checks, compaction logic, retry logic, and error handling. This means:
- Every new feature must be implemented twice
- The continuation path has a fallback retry that calls the model AGAIN if the first attempt fails (lines 874-1039)
- The continuation path has a compaction block (lines 716-810) that mirrors the initial path's compaction block (lines 1179-1271)

**Recommendation**: Unify the two branches. The TypeScript approach of just appending messages and letting the same loop iteration handle it is correct. The deep-code split exists primarily because the initial request uses a different message format (system + user) vs continuation (system + user + tool evidence), but this can be unified by constructing messages BEFORE the model call rather than branching the entire iteration.

---

### 2. Event Construction: HIGH gap

**OCC**: Event/message construction is centralized in `utils/messages.js`:
- `createUserMessage()` (line 46)
- `createSystemMessage()` (line 46)
- `createAssistantAPIErrorMessage()` (line 46)
- `createUserInterruptionMessage()` (line 46)
- `createAttachmentMessage()` (line 60)

Each returns a typed `Message` object with a stable schema.

**deep-code**: Events are constructed ad-hoc via inline `format!()` calls throughout the 2300-line loop. For example:
```rust
// native_agent_loop.rs line 262-276
format!(
    "{{\"call_id\":{},\"retry_call_id\":{},\"stage\":{},\"compacted_stage\":{},\"status\":\"compacted\",\"marker\":{},\"token_estimate_before\":{},\"token_estimate_after\":{},\"prompt_tokens_after_injection\":{},\"target_limit_tokens\":{},\"compaction_reason\":{},\"summary\":{}}}",
    json_string(&guard_report.call_id),
    ...
)
```

There are approximately 50+ separate `session.record_runtime_event()` calls with inline JSON construction scattered across the loop. This is fragile (syntax errors in JSON templates), untyped, and impossible to refactor safely.

**Recommendation**: Extract event construction into a dedicated module with typed builder functions or structs, similar to `utils/messages.js`. This would also make the loop body much shorter and testable in isolation.

---

### 3. Cross-Iteration State Management: HIGH gap

**OCC** (`query.ts` lines 204-217): Single `State` type with 9 fields that is atomically replaced at each continue site:
```typescript
type State = {
  messages, toolUseContext, autoCompactTracking, maxOutputTokensRecoveryCount,
  hasAttemptedReactiveCompact, maxOutputTokensOverride, pendingToolUseSummary,
  stopHookActive, turnCount, transition
}
// Continue sites:
state = { ...state, messages: [...], transition: { reason: 'next_turn' } }
```

Each transition includes a `reason` field making the loop's decision history traceable via `state.transition`.

**deep-code**: Cross-iteration state is scattered across mutable local variables in `run_native_agent_loop_v2_deepseek_inner()`:
- `last_tool_batch` (line 529)
- `evidence_ledger` (line 535)
- `repeated_tool_contract_failures` (line 541)
- `deepseek_cache_zone_telemetry` (line 542)
- `reasoning_replay` (line 543)
- `error_recovery` (line 548)
- `active_max_tokens` (line 549)
- `output_truncation_recovery` (line 552)
- `deepseek_adaptation` / `dual_protocol` (lines 553-554)
- `tool_call_count` / `model_call_count` (lines 520-521)
- `turn_state` (line 522)
- `turn_controller` (line 506)

There is no equivalent of `transition.reason` in deep-code -- the loop just `continue`s without recording why. This makes debugging and testing recovery paths significantly harder.

**Recommendation**: Extract a `NativeLoopState` struct analogous to OCC's `State` type. Replace the 12+ mutable local variables with atomic state transitions at continue sites.

---

### 4. Turn Controller Over-Engineering: HIGH

**OCC**: Has no explicit turn controller abstraction. The `while(true)` loop manages everything. Iteration guard is implicit (maxTurns check at line 1705). Progress tracking is implicit (tool use blocks -> needsFollowUp).

**deep-code**: Has THREE separate turn controller abstractions:

1. **`TurnController<Ctx>` trait** (`agent_kernel/turn_controller.rs` line 25): Generic trait with `fn run_iteration(&mut self, ctx: &mut Ctx) -> IterationOutcome`. Implemented once for `NativeLoopTurnController`.

2. **`NativeLoopTurnController`** (`agent_kernel/turn_controller.rs` lines 164-1253, ~1090 lines): Owns iteration preflight gates, convergence decisions, tool batch signature tracking, continuation strategy selection, progress reporting, DSML leak tracking, and non-progress recovery. Has 25+ methods.

3. **`NativeTurnController`** (`native_turn_controller.rs` lines 45-288, ~243 lines): Owns turn-level tool/permission ledger (pending_tools, completed_tools, pending_permissions). Used as a separate object in the main loop.

These three layers overlap. For example:
- `NativeLoopTurnController.observe_completed_tool_iteration()` (line 834) records `turn.convergence.decision` events
- `NativeTurnController.record_tool_completed()` (line 177) records `agent.tool.completed` events
- Both feed into `turn_state.observation_cache` and `turn_state.progress`

**Recommendation**: Merge `NativeTurnController` (the ledger) into `NativeLoopTurnController`. The separation between "kernel turn controller" and "native turn controller" adds complexity without clear benefit -- they are both used exclusively by the native loop.

---

### 5. Compaction Pipeline: MEDIUM gap

**OCC** (`query.ts` lines 396-543): Multi-stage compaction pipeline executed in order at the top of each iteration:
1. **Tool result budget** (line 379): Truncates oversized tool results
2. **Snip compaction** (line 397): Removes old tool messages
3. **Micro-compact** (line 413): Trims tool results to a budget
4. **Context collapse** (line 440): Projects collapsed context view
5. **Auto-compact** (line 454): Full context compaction to summary
6. **Reactive compact** (line 1119): Triggered by API 413 errors

**deep-code** (`native_turn_controller.rs` lines 290-424): Single compaction strategy:
- `evaluate_native_context_guard()` checks if estimated tokens exceed threshold
- If so: `Compactor::compact()` generates a summary, then the request is rebuilt
- No equivalent of: snip, micro-compact, context collapse, reactive compact, or tool result budget

This is partly justified because deep-code targets DeepSeek's 256K context window (vs OCC's typical 200K Anthropic window), and deep-code's continuation strategy clears tool batches each iteration (lines 1705-1706) to prevent context growth. However, the lack of a reactive compaction path (retry after 413) is a real gap.

**Recommendation**: Add a reactive compaction path that triggers on HTTP failure (already partially done at lines 874-1039 for continuation HTTP failures, but missing for initial request failures).

---

### 6. Continuation Strategy: Architectural divergence (by design)

**OCC**: Continuation is simple -- accumulated messages are sent back to the API. The Anthropic API natively understands `tool_use` -> `tool_result` pairing.

**deep-code**: Continuation has TWO strategies with explicit fallback:

1. **ProviderToolResult** (`native_agent_loop_continuation.rs` lines 266-341): Replays tool calls and results using the provider's native format (Anthropic-style `tool_use`/`tool_result` blocks or OpenAI-style `tool_calls`/`tool` messages). This is the preferred path.

2. **PlainEvidence** (`native_agent_loop_continuation.rs` lines 100-150): Falls back to plain text evidence when the provider rejects structured tool_result replay. Converts tool results into a text block like "Already Executed Tool Evidence".

This complexity is NECESSARY for DeepSeek models. DeepSeek's API does not consistently support the Anthropic-style tool_result replay. The fallback mechanism at lines 874-1039 in the main loop (where a failed ProviderToolResult continuation is retried as PlainEvidence) is architecturally correct.

However, `ContinuationStrategy::select_continuation_strategy()` at `agent_kernel/turn_controller.rs` line 588-595 always returns `ProviderToolResult` regardless of family/protocol:
```rust
pub fn select_continuation_strategy(&self, family: &NativeModelFamily, protocol: &str) -> ContinuationStrategy {
    let _ = (family, protocol);
    ContinuationStrategy::ProviderToolResult
}
```
The comments indicate this is intentional (prefer provider-native, fallback on failure), but the unused parameters suggest this was designed for model-aware selection that was never implemented.

---

### 7. Resume/Entry Points: HIGH gap

**OCC** (`QueryEngine.ts`): Single `submitMessage()` method. Resume is implicit -- the engine holds `mutableMessages` across calls. Each `submitMessage()` call starts a new turn within the existing conversation.

**deep-code** (`native_agent_loop_entrypoints.rs`): Four entry points:
- `run_native_agent_loop_v2_deepseek()` (line 112)
- `run_native_agent_loop_v2_deepseek_with_event_sink()` (line 119)
- `run_native_agent_loop_v2_deepseek_with_interrupt()` (line 132)
- `run_native_agent_loop_v2_deepseek_resume()` (line 142-497)

The resume path (`_deepseek_resume`, lines 142-497) is a separate 355-line function that reimplements a simplified version of the main loop. It has its own:
- Session construction from event log (lines 146-149)
- Manifest building (lines 161-174)
- Tool execution via `send_with_live_visible_stream_events` (lines 289-301)
- Iteration loop (lines 341-496)

Critically, the resume path LACKS:
- Compaction guards (no `guard_native_loop_prepared_request_report`)
- Convergence checks (no `ConvergenceEnforcer`)
- Observation cache integration
- Permission mode handling
- Tool exposure escalation
- Repeated tool contract failure tracking
- Error recovery state
- Output truncation recovery
- DeepSeek adaptation manager / dual protocol fallback
- DSML leak detection
- Transition statement detection
- Batch guard / synthetic recovery
- Duplicate observation suppression

This means resumed sessions are significantly less robust than fresh sessions.

**Recommendation**: The resume path should share the main loop infrastructure rather than duplicating a subset of it. The `NativeAgentLoopV2ResumeRequest` should be convertible into the standard `NativeAgentLoopV2Request` flow, or the inner loop (`run_native_agent_loop_v2_deepseek_inner`) should accept a "resume from" state parameter.

---

### 8. Missing Abstractions Summary

| Abstraction | OCC | deep-code | Gap |
|---|---|---|---|
| State object | `State` type, 9 fields | 12+ scattered mut locals | HIGH |
| Transition tracking | `transition.reason` | None | HIGH |
| QueryConfig snapshot | `buildQueryConfig()` | None | MEDIUM |
| Message construction helpers | `createUserMessage()`, etc. | Inline `format!()` JSON | HIGH |
| Stop hooks | `handleStopHooks()` | None | MEDIUM |
| Token budget tracking | `createBudgetTracker()`/`checkTokenBudget()` | `TurnBudget` (static) | MEDIUM |
| Streaming tool executor | `StreamingToolExecutor` | Inline stream handler | LOW |
| Post-sampling hooks | `executePostSamplingHooks()` | None | LOW |
| Reactive compaction | `reactiveCompact.tryReactiveCompact()` | None (partial HTTP retry) | MEDIUM |
| Context collapse | `contextCollapse.applyCollapsesIfNeeded()` | None | LOW |
| Snip compaction | `snipModule.snipCompactIfNeeded()` | None | LOW |
| Micro-compact | `deps.microcompact()` | None | LOW |
| Tool-use summary | `generateToolUseSummary()` | None | LOW |
| Skill discovery prefetch | `startSkillDiscoveryPrefetch()` | None | LOW |
| Memory prefetch | `startRelevantMemoryPrefetch()` | None | LOW |
| Task summary (bg) | `maybeGenerateTaskSummary()` | None | LOW |

---

### Gap Severity Summary

| Severity | Count | Items |
|---|---|---|
| **CRITICAL** | 1 | Control flow: two separate code paths for initial vs continuation merged into one function |
| **HIGH** | 4 | No cross-iteration State type; event construction via inline JSON; three turn controller layers; resume path missing safety checks |
| **MEDIUM** | 4 | No reactive compaction; no transition tracking; no QueryConfig snapshot; no stop hooks |
| **LOW** | 7 | Missing snip/micro-compact/collapse compaction strategies; no streaming tool executor; no tool-use summary; no skill/memory prefetch; no task summary |

---

### Specific Action Items

1. **`native_agent_loop.rs` lines 627-1079 and 1080-1401**: Unify the initial request and continuation paths. Construct the message array BEFORE the model call decision point rather than duplicating the entire call+response+tool-execution pipeline.

2. **`native_agent_loop.rs` lines 529-554**: Extract a `NativeLoopState` struct to hold all cross-iteration mutable state. Replace inline mutations with atomic state transitions at continue points. Add a `transition_reason` field.

3. **`native_agent_loop.rs` (entire file)**: Extract all `session.record_runtime_event(format!(...))` calls into named event constructors in a dedicated module (e.g., `native_agent_loop_events.rs`).

4. **`native_agent_loop_entrypoints.rs` lines 142-497**: Refactor `run_native_agent_loop_v2_deepseek_resume` to reuse `run_native_agent_loop_v2_deepseek_inner` instead of duplicating the loop. The resume request should hydrate into a standard request with pre-populated state.

5. **`agent_kernel/turn_controller.rs` and `native_turn_controller.rs`**: Merge `NativeTurnController` (ledger) into `NativeLoopTurnController`. The trait `TurnController<Ctx>` adds value for testing but the two concrete structs are always used together by the same consumer.

6. **`agent_kernel/turn_controller.rs` lines 588-595**: Either implement model-aware continuation strategy selection using the `family`/`protocol` parameters, or remove them and simplify the signature to `fn select_continuation_strategy() -> ContinuationStrategy`.

7. **`native_turn_controller.rs` lines 290-424**: Add a reactive compaction path triggered by HTTP 413/400 errors in the initial request path (currently only the continuation path retries, lines 874-1039).