# Red Team Report: Robustness and Edge Cases

**Date:** 2026-06-03
**Reviewer:** Red Team (robustness focus)
**Scope:** Phase 1 audit reports + relevant source code

---

## Severity Re-Ratings

These are existing issue-matrix entries whose severity is incorrect based on robustness analysis.

---

### RR-1: P2-20 (Convergence Loop Not Self-Terminating) should be P0

**Original rating:** P2
**Correct rating:** P0
**Rationale:** The ConvergenceEnforcer (`agent_kernel/convergence_enforcer.rs:42-48`) only produces non-Continue verdicts for `DuplicateDominance`, `InformationStagnation`, and `BatchNoveltyPlateau`. These are all mapped through `observe_convergence` in `turn_controller.rs:985-1010`, which returns `LoopConvergenceAction::Continue` / `SoftWarning` / `EscalateToCodeEdit` -- it **never returns Stop**. The only mechanism producing Stop is `ToolProgressState::record_error_signature` at `turn_state.rs:108-118`, which requires 6 identical error signatures. All other plateau conditions (duplicate observation, non-progress, convergence stall) produce warnings that do not terminate the loop. Combined with the `SoftWarning` handler in `native_agent_loop.rs:2811-2813` which just calls `emit_new_session_events` and falls through to Continue, the loop runs until:
- `max_iterations` budget exhaustion (8 by default, but apparently not enforced in all paths), or
- 6 identical tool errors (the `SAME_TOOL_ERROR_STOP_STREAK`), or
- External cancellation

This explains the 70-iteration plateau observed in regression data (report 16, Finding 4). The loop does not self-terminate for non-error plateaus.

**Suggested fix:** Make `ObserveConvergence::InformationStagnation` produce Stop after the no-new-evidence window, not just SoftWarning. Add a hard cap on consecutive SoftWarning iterations (e.g., 5 soft warnings = forced stop).

---

### RR-2: Agent 05 P2 (json_object_complete) should be P1

**Original rating:** P2
**Correct rating:** P1
**Rationale:** `json_object_complete` (`stream.rs:891-925`) accepts `{}` and any syntactically balanced JSON regardless of semantic content. Combined with `completed_call_if_ready` (`stream.rs:856-888`), which uses `json_object_complete` as the sole completion gate for `StreamingToolCallAssembler`, this means:
1. A `tool_use` content block with `input: {}` emits a CompletedStreamingToolCall with empty arguments
2. That call goes through `mediate_tool_call_with_provider_id` which rejects it for missing required fields
3. The rejection creates a model-readable error that consumes tool call budget
4. The model sees this error and (per Finding 2 in report 16) often repeats the same call

The issue is framed as P2 because the "system handles it eventually," but the budget consumption and model confusion is a correctness problem. A tool call with `arguments: "{}"` should not be emitted as assembled -- it should wait for more arguments or be rejected at assembly-time with a specific error.

**Suggested fix:** In `completed_call_if_ready`, add a `ToolCallParseStatus::Incomplete` status when arguments are `null` or `{}`. Do not emit a completed call. Alternatively, add a timeout for incomplete calls in `complete_stream()`.

---

### RR-3: Agent 12 P2-13 (Narrative Text Swallowed) should be P1

**Original rating:** P2
**Correct rating:** P1
**Rationale:** The `terminalStreamClosedRef` mechanism (`useRuntimeEventApplication.ts:197-206`) drops narrative text emitted as `model.stream_delta` between tool turns. This is an observable correctness issue: the model can emit text like "Let me check the result..." after a tool call completes but before the next model call starts, and this text is silently discarded. The report correctly identifies this but underestimates impact -- this text often contains model reasoning about which tool to call next. Losing it makes the transcript incomplete, which in turn corrupts conversation context for continuation/compaction.

The fix proposed (reset on `model.call_started` before checking) is correct but incomplete: the 220ms `scheduleStreamCommit` timer creates a race window. If a new `model.call_started` arrives within 220ms of `model.stream_completed`, the timer fires AFTER the ref is reset, re-setting it to true and dropping early deltas of the new call.

**Suggested fix:** Use a generation counter (increment on each `model.call_started`) to match commit timers to the correct stream generation.

---

## New Edge Case Findings

---

### EC-1: stream_event_handler Emits Dirty Events Before HTTP Status Confirmed

**Severity:** P0
**Files:** `native_agent_loop_model_io.rs:448-509`
**Edge case:** During dual-protocol streaming, the `stream_event_handler` closure (invoked at line 503) is called regardless of whether the HTTP response succeeded. The code comment at lines 414-422 claims "The observer buffers visible/thinking deltas and flushes them only after a 2xx response is confirmed." However, the `stream_event_handler` call at lines 503-509 is NOT gated on `live_stream_status_is_success`. It fires for every event including `ToolCallStarted`, `ToolCallFinished`, `ContentBlockStarted`, and `ContentBlockFinished`.

**What goes wrong:** When the Anthropic endpoint returns 400 and dual-protocol fallback retries on OpenAI, the first attempt's structural events have already been dispatched to `stream_event_handler`. The handler records tool calls in the turn controller. When the OpenAI retry succeeds and emits a second set of tool call events, the turn controller's duplicate detection fires, producing `duplicate tool_call_id in native turn ledger` (the error observed in regression Finding 9, session `1779076559811220000`).

The existing mitigation buffers only `VisibleTextDelta` and `ThinkingDelta` events (lines 462-465) -- structural/tool events pass through immediately. This is the root cause of the dirty-streaming issue flagged in agent 04 Finding 2 (P1-07).

**Suggested fix:** Gate the entire `stream_event_handler` call on `live_stream_status_is_success`. Buffer ALL events (not just text deltas) and replay them after the 2xx confirmation. Alternatively, add a `reset()` method to the turn controller and stream processor to unwind state when a retry occurs.

---

### EC-2: merge_events Side-Effect Applier Bypasses State Transition Validation

**Severity:** P1
**Files:** `session.rs:1032-1081` (`apply_merged_event_side_effect`)
**Edge case:** When events from a completed agent loop are merged via `merge_events_with_id_suffix`, the `apply_merged_event_side_effect` method (line 1032) directly sets `self.state` (line 1041) without calling `can_transition`. This means an event from the merged log that records a state change from `Executing` to `Created` (which is not a valid transition in `state.rs:21-74`) would be applied silently, putting the session into an unreachable state.

**What goes wrong:** If the merged event log contains a `session.state_changed` event with `to: "Planning"`, and the current session is in `Executing`, the state becomes `Planning` even though `Executing → Planning` is NOT a valid transition. Subsequent operations that rely on state-dependent behavior (e.g., `begin_interactive_turn` at line 208, `submit_plan_decision` at line 322) will behave incorrectly.

**Suggested fix:** Validate the transition with `can_transition(current_state, next_state)` before applying the side effect. If invalid, log a warning and skip the state assignment (but still merge the event for audit purposes).

---

### EC-3: Sidecar Interrupt Discards Already-Parsed Output

**Severity:** P1
**Files:** `sidecar_http_transport.rs:305-308`, `sidecar_http_transport.rs:430-442`
**Edge case:** When the interrupt flag is set (line 305-308), the child process is killed and the function returns `Err("sidecar_interrupted")`. All output lines already parsed from the sidecar's stdout are discarded -- `output.stdout_tail` and any tool call events emitted to the observer are lost.

**What goes wrong:** The sidecar process (Python Qwen runner) may have already successfully parsed and emitted valid tool calls. These tool calls are discarded when the error is propagated up. The caller receives nothing and the turn fails with a generic error, even though the model produced valid tool calls that were incompletely delivered.

A secondary issue: when the sidecar process exits with non-zero status (line 436-438), the function returns an error even if `output` contains valid parsed content. The `output` is discarded -- `output.status_code`, `output.http_error_preview`, and `output.skipped_reason` are never inspected in the error path.

**Suggested fix:** Return the `StreamingSidecarOutput` alongside the error so the caller can inspect `output.skipped_reason` and decide whether to use partial results. Add a `partial_output` field to the error format.

---

### EC-4: DsmlChunkFilter.pending Unbounded Growth

**Severity:** P2
**Files:** `stream.rs:716-783` (`DsmlChunkFilter`)
**Edge case:** If the model emits an opening DSML marker (e.g., `<tool_call>`) and never closes it, the `DsmlChunkFilter` enters the `inside = true` state. In the `inside` branch (lines 734-756), when no end marker is found, `split_pending_marker_prefix` sets `self.pending` to a suffix of the current chunk. On each subsequent chunk, the pending buffer grows by `self.pending.push_str(chunk)` at line 728. There is no maximum size limit on `self.pending`.

**What goes wrong:** Over a long stream, if the model's output contains a DSML tag that never closes (e.g., malformed reasoning output), `self.pending` grows proportionally to the total stream length. For a 100K-character stream with an unclosed tag at character 1000, `self.pending` accumulates approximately 99K characters. This is invisible to the user (all output is suppressed) and the memory is never released until the stream ends.

**Suggested fix:** Add a maximum pending buffer size (e.g., 4096 chars). If exceeded, force `inside = false`, emit a `dsml.filter_overflow` telemetry event, and resume normal output.

---

### EC-5: StreamProcessor.raw_visible_content Unbounded Accumulation

**Severity:** P2
**Files:** `stream_processor.rs:62-68` (fields), `stream_processor.rs:159-334` (accumulation logic)
**Edge case:** The `StreamProcessor` stores all visible text content in `state.raw_visible_content` for the entire duration of the stream. This content is used in `complete_stream()` (line 340) to scan for content tool call candidates only when `had_tool_call` is false. However, it accumulates ALL visible deltas regardless.

**What goes wrong:** For a long DeepSeek reasoning response with 50K+ characters of visible text, `raw_visible_content` grows to 50K+ characters even though the scan in `complete_stream()` only matters when `had_tool_call` is false. If a tool call is present, the scan is skipped (line 337: `if self.state.had_tool_call ... return default()`). This means in the most common case (stream with tool calls), the 50K buffer is retained for the entire stream lifetime without ever being used.

**Suggested fix:** Clear `raw_visible_content` as soon as `had_tool_call` becomes true. Keep only a truncated tail (e.g., last 1000 chars) for telemetry.

---

### EC-6: ToolProgressState.reset_repeated_error_streak Resets on Any Success

**Severity:** P2
**Files:** `turn_state.rs:127-130`, `turn_state.rs:145`
**Edge case:** `reset_repeated_error_streak` is called in two places:
1. `record_successful_tool_results` (line 122-124): resets on any successful result
2. `record_iteration` (line 145): resets on any new observation

This means if the model alternates between a successful `file.list_directory` and a failing `file.edit` with the same error, the error streak NEVER reaches `SAME_TOOL_ERROR_STOP_STREAK = 6`. The successful interleaved call continuously resets the counter.

**What goes wrong:** The model can make the same erroneous `file.edit` call 100 times, each preceded by a successful `file.list_directory`, and the plateau detector never triggers. This is a disguised form of the 70-iteration plateau observed in regression data -- the model may be interleaving a cheap successful call with a repeated failed call to defeat the error streak counter.

**Suggested fix:** Track error signatures independently per tool. Reset only the streak for tools that succeed, not the global streak. Or use a per-tool repeated-error counter that doesn't reset on successes of other tools.

---

### EC-7: extract_json_string Misparses Consecutive Escape Sequences

**Severity:** P2
**Files:** `session.rs:1163-1174` (`extract_json_string`)
**Edge case:** The `extract_json_string` function uses a placeholder-character replacement strategy:
```
.replace("\\\\", "\x00")  // escaped backslash → placeholder
.replace("\\\"", "\"")     // escaped quote → unescaped quote
.replace("\x00", "\\")     // placeholder → literal backslash
```
Consider the JSON string `"abc\\\"def"` (JSON escaping: `abc`, literal backslash, escaped quote, `def`). The raw text between JSON quotes is `abc\\\"def`. Processing:
1. `abc\\\"def`.replace("\\\\", "\x00") → `abc\x00\"def`
2. `abc\x00\"def`.replace("\\\"", "\"") → `abc\x00"def`
3. `abc\x00"def`.replace("\x00", "\\") → `abc\"def`
Result: `abc\"def` -- correct.

But consider `"abc\\\\\"def"` (backslash, backslash, escaped-quote):
Raw: `abc\\\\\"def`
1. `abc\x00\x00\"def`
2. `abc\x00\x00"def` (the `\"` after the two placeholders is now `"`)
3. `abc\\"def`
Result: `abc\\"def` -- correct, this is two backslashes followed by a quote.

Now consider `"abc\\u0022def"` (backslash, unicode escape for quote):
Raw: `abc\\u0022def`
1. `abc\x00u0022def`
2. The `\"` matcher doesn't fire here, so: `abc\x00u0022def`
3. `abc"def`
Result: `abc"def` -- correct, the unicode escape is preserved.

The algorithm is actually more robust than it first appears. However, the edge case remains:
- The `.find(&marker)` call at line 1165 uses simple substring search, not JSON-aware search. If a string value elsewhere in the JSON contains the marker pattern (e.g., a value like `"key\":\"value"`), the function would match it. This is unlikely in practice but is a correctness gap.

**Finding severity downgraded:** This is a latent fragility (P3) rather than an active bug. No fix needed unless hitting production issues.

---

### EC-8: cancel_session Leaves Stale pending_native_decision

**Severity:** P1 (confirms and extends P1-05)
**Files:** `runtime_facade_impl.rs:54-79`, `runtime_facade_impl.rs:380`
**Edge case:** When `cancel_session` is called (line 54), it sets the interrupt flag and transitions the session to `Cancelled`. However, `pending_native_decision` is NOT cleared. The field is only cleared in `runtime_facade_impl.rs:424` and `runtime_facade_impl.rs:584` -- both in the resume path, not the cancel path.

**What goes wrong:** If a new turn starts on the same session (after `Cancelled` → `begin_interactive_turn` bypasses `can_transition` at line 249, setting state to `Executing`), the stale `pending_native_decision` persists. The next call to check whether a turn is in progress (line 3603: `record.pending_native_decision.is_none()`) would return false, preventing the new turn from starting.

**Suggested fix:** Clear `pending_native_decision` in the `cancel_session` method when the session is already in `Executing` state (indicating an active turn). Add a test that cancels a session mid-turn and then starts a new turn.

---

### EC-9: plateau_fallback_notes Filtered but Still Consume Context Budget

**Severity:** P3
**Files:** `runtime_facade_impl.rs:3566`, `context_service.rs:60`
**Edge case:** The `is_plateau_fallback_note` filter at `context_service.rs:60` prevents plateau fallback notes from being injected into session memory. However, the fallback note is generated and the entire structured failure message (including all tool results) is still serialized into the event log before the filter runs.

**What goes wrong:** The fallback note event is recorded in the session event log (consuming storage) and transmitted to the UI (consuming bandwidth) before being filtered from memory. For very long session histories, these filtered events silently bloat the event log. No observable user impact, but a resource inefficiency that adds up over long sessions.

---

### EC-10: complete_stream Does Not Flush StreamingToolCallAssembler

**Severity:** P1 (confirms Agent 05 P1 finding)
**Files:** `stream_processor.rs:336-351`, `stream.rs:856-888`
**Edge case:** When `complete_stream()` is called, it only scans `raw_visible_content` for content tool call candidates. It does NOT call any method on the `StreamingToolCallAssembler` (held in `self.tool_pipeline.streaming`) to flush incomplete tool calls. The Agent 05 report found this as a P1, confirmed.

**Additional finding:** Even when `json_object_complete` returns false indicating an incomplete JSON object, the function returns `None`. The `StreamingToolCallAssembler` retains the partial state in its `calls: BTreeMap<usize, StreamingToolCallState>`, but no code path ever reads it after `complete_stream()`. The partial tool call is lost with zero observability -- no error event, no telemetry, no log message.

**Additional mitigation gap:** The `finish_reason` (`DeepSeekStreamDelta::StopReason`) at `stream_processor.rs:132-134` is parsed but matched to `_ => {}`. The reason is completely discarded. If the stream ended with `finish_reason: "tool_calls"` but the tool call arguments were incomplete, the incomplete call is dropped and the reason for stream end is lost.

**Suggested fix:** In `complete_stream()`, iterate over `StreamingToolCallAssembler`'s uncompleted calls and emit `ToolCallParseError` events for each one. Include `finish_reason` in the `StreamCompleted` event. Log a warning when incomplete tool calls are detected.

---

### EC-11: Dual-Protocol Retry Loop Transient Count Not Reset After Success

**Severity:** P3
**Files:** `native_agent_loop_model_io.rs:339-409`, `native_agent_loop_model_io.rs:428-748`
**Edge case:** The `transient_retry_count` and `transient_attempt` variables are reset to 0 at the top of each loop call (lines 330, 397 for non-streaming; similar for streaming). However, the `ErrorRecovery.max_tokens` escalation state persists across calls (it's part of the `ErrorRecovery` struct passed by reference). If a previous call escalated `max_tokens` and the current call succeeds immediately, the escalated state persists into the next call. The `on_success()` method is called at lines 362 and 738 to clear it, but only when the response is in the 200 range.

**What goes wrong:** If a call succeeds on the first attempt without any retries, `on_success()` is still called, so this works. The issue is that between calls, if no explicit reset occurs, the escalated `max_tokens` value from a previous transient retry could leak into a new call's initial request. The `ErrorRecovery` is per-loop-iteration, so this only matters if the same error recovery struct is reused across iterations. This appears to be mitigated by re-creating the error recovery each iteration, but the code is not self-documenting.

---

### EC-12: Permission Resume Race Window Between Decision and Execution

**Severity:** P1 (confirms P1-04)
**Files:** `runtime_facade_impl.rs` (permission submission path)
**Edge case:** There is a race between `submit_permission_decision` and the session mutex acquisition. The report calls this an "unlocked window" between `decide_permission()` and `execute_tool()`. If the session is cancelled between the permission decision being recorded and the tool execution starting, the tool runs against a cancelled session state.

**Additional finding not in original report:** The `pending_native_decision` holds a `session_id` reference but no generation counter. If a session is cancelled and a new turn starts with the same session_id, the stale `pending_native_decision` from the cancelled turn could be mistaken for an active decision on the new turn. The check at line 3603 (`record.pending_native_decision.is_none()`) uses presence/absence, not a generation counter.

**Suggested fix:** Add a `turn_id` to `RuntimePendingNativeDecision` and validate it matches the current turn before acting on it.

---

### EC-13: SAME_TOOL_ERROR_STOP_STREAK = 6 Is Too Conservative

**Severity:** P2
**Files:** `turn_state.rs:92` (`const SAME_TOOL_ERROR_STOP_STREAK: u32 = 6`)
**Edge case:** The stop threshold of 6 identical errors from the same tool requires the model to make the exact same error 6 consecutive times without any successful tool calls of the same type. With a `max_iterations` budget of 8 and typical tool batch sizes of 2-4 tools per iteration, 6 errors means the model wastes between 1.5 and 3 full iterations before the plateau detector acts.

**What goes wrong:** Combined with EC-6 (successful interleaved calls reset the streak), the effective number of identical errors before stopping can be much higher than 6. The model can waste most of the turn budget on repeated validation errors before the plateau detector intervenes.

**Suggested fix:** Reduce `SAME_TOOL_ERROR_STOP_STREAK` to 4. Add a per-tool counter that doesn't reset on unrelated tool successes. Add a global counter of total-error-iterations-without-progress that forces a stop after 5 consecutive error-only iterations regardless of signature diversity.

---

### EC-14: Qwen Sidecar Silent Failure Masks Real Errors

**Severity:** P1 (confirms P3-10, argues it should be P1)
**Files:** `sidecar_http_transport.rs:436-438`, `regression report Finding 13`
**Edge case:** When the Qwen sidecar process exits with a non-zero status, `run_sidecar_streaming_process` returns `Err(format!("sidecar_failed: {status}"))`. The caller receives this error, but the session may still reach `Completed` state (as observed in session `1778840437599190000`).

**What goes wrong:** The regression data shows this error occurring but the session reaching `Completed` anyway. This implies the caller treats the sidecar error as non-fatal or has a fallback path that silently succeeds with empty content. The user sees a completed session with potentially no visible output -- the failure is invisible. All three rounds in the observed session had `sidecar_failed: exit status: 1`.

**Suggested fix:** When the sidecar process fails, emit a `model.call_blocked` event with the error code `sidecar_failed` and the exit status. Ensure the session transitions to `Failed` (not `Completed`) when all model calls in a turn fail via sidecar errors.

---

## Test Coverage Gaps

### Critical Untested Paths

1. **Ungated stream_event_handler on dual-protocol fallback:** No test sends an Anthropic request, gets 400, retries on OpenAI, and verifies that tool call events from the failed attempt are NOT visible in the event log. The current dual-protocol test likely only checks the successful path.

2. **merge_events invalid state transition:** No test provides a merged event log containing a `session.state_changed` with an invalid transition and verifies the resulting session state is rejected or warned.

3. **complete_stream with incomplete tool calls:** No test ends a stream with partial tool call arguments in `StreamingToolCallAssembler` and verifies the calls are handled (flushed or rejected with observability).

4. **DsmlChunkFilter.max_pending overflow:** No test feeds a 100KB stream with an unclosed DSML tag and verifies memory behavior.

5. **Interleaved error/success streak reset:** No test for `ToolProgressState` where alternating success on tool A and repeated error on tool B causes the error streak to never reach the stop threshold.

6. **cancel_session with pending_native_decision:** No test cancels a session mid-turn and then starts a new turn to verify the stale decision doesn't block the new turn.

7. **Sidecar interrupt mid-output:** No test kills a sidecar process after it sent half of a tool call and verifies that the partial output is handled.

8. **permission_id and plan_approval_id collision in merge_events:** No test merges two loop invocations where both produce permission requests with the same ID and verifies no collision.

9. **finish_reason: "tool_calls" with incomplete arguments:** No test simulates a stream where the provider sends `finish_reason: "tool_calls"` before completing the tool call arguments JSON.

10. **StreamProcessor concurrent retry and event emission:** No test verifies that re-creating the `StreamProcessor` for a retry doesn't retain state from the failed attempt (the `stream_processor` is created inside the outer retry loop at line 447, inside `loop`, so it IS re-created per retry -- but this isn't tested explicitly).

---

## Regression Event Data Re-Interpretation

### Finding 1 (HTTP 400 infinite retry) -- PARTIALLY CORRECT

The `model.call_blocked` events with `gate: "http_status_400"` are correctly identified. However, the mechanism is not an "infinite retry" in the HTTP transport layer -- 400 is NOT in the transient retry list (`native_agent_loop_model_io.rs:753-756`), so a 400 response is returned to the loop as a failure. The "infinite" pattern occurs because:
1. HTTP call returns 400 → loop records `model.call_blocked` → loop fails iteration
2. Either loop continues with retry budget (iteration_count < max_iterations) or stops
3. If it continues, next iteration makes a new HTTP call with similar content → gets 400 again

The loop-level retry uses the same `max_iterations` budget, so it's bounded (capped at 8 by default, not infinite). The "15 occurrences across 7 sessions" is consistent with ~2 failed iterations per session.

### Finding 4 (70-iteration plateau) -- VERIFIED as DESIGN FLAW

Per EC-1 and RR-1, the 70-iteration delay is not a tuning issue (threshold too high) -- it's a structural issue where `SoftWarning` does not stop the loop. The only backstop is `SAME_TOOL_ERROR_STOP_STREAK = 6`, which requires identical error signatures. If the model produces varied (but equally useless) errors, the plateau detector never fires Stop.

### Finding 9 (duplicate tool_call_id) -- ROOT CAUSE IDENTIFIED

Per EC-1, the `duplicate tool_call_id in native turn ledger` error in session `1779076559811220000` is caused by dirty structural events on dual-protocol fallback. The events from the failed Anthropic attempt enter the turn controller, and when the OpenAI retry emits the same tool calls, the controller detects duplicates. This matches the error message format and the model being DeepSeek (which uses dual-protocol).

---

## Summary

| Severity | Count | Items |
|----------|-------|-------|
| P0 | 2 | EC-1 (dirty events on fallback), RR-1 (convergence never stops) |
| P1 | 7 | EC-2 (merge bypasses transitions), EC-3 (sidecar discard), EC-8 (stale pending_decision), EC-10 (incomplete tool call loss), EC-12 (permission race), EC-14 (Qwen silent failure), RR-2 (json_object_complete) |
| P2 | 5 | EC-4 (DsmlFilter overflow), EC-5 (raw_visible_content), EC-6 (interleaved error reset), EC-13 (threshold too high), RR-3 (narrative swallow) |
| P3 | 2 | EC-7 (extract_json_string), EC-11 (transient count) |

**Key takeaway:** The three most critical issues are:
1. **EC-1**: Structural tool events leak to the turn controller on dual-protocol fallback, causing duplicate-detection failures
2. **RR-1**: The convergence enforcer and plateau detector produce warnings but never force a stop for non-error plateaus
3. **EC-6**: Interleaved successful calls reset the error streak counter, defeating the only mechanism that CAN stop the loop
