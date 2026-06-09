Now I have a comprehensive understanding of all the files. Here is the full line-by-line review.

---

## Code Review: native_agent_loop_execution.rs and native_agent_loop_model_io.rs

### FILE 1: `native_agent_loop_execution.rs` (1025 lines)

---

#### Bug Count by Severity

| Severity | Count |
|----------|-------|
| CRITICAL | 0 |
| HIGH    | 1 |
| MEDIUM  | 3 |
| LOW     | 3 |

#### Issues Found

---

**Issue 1: Missing batch-result-injection for tool_use blocks without results**

- **Line range**: N/A (missing function)
- **Severity**: HIGH
- **Description**: The TypeScript reference (`query.ts` lines 123-149) defines `yieldMissingToolResultBlocks` which yields synthetic `tool_result` error blocks for every `tool_use` in a batch of assistant messages. This is essential for recovery paths where the model yielded tool_use blocks but the turn fails before results are generated (e.g., model fallback, API error). Without this, the conversation history accumulates orphaned `tool_use` blocks with no matching `tool_result`, which causes API errors on the next request. The Rust code has `execute_model_readable_error_collect` (line 170) for single-tool errors, but no batch-level equivalent.
- **Reference**: `query.ts` lines 896-903 (fallback path), lines 980-993 (general error path).
- **Fix**: Add a function `yield_missing_tool_result_blocks(session, assistant_messages, error_message)` that iterates over all `tool_use` blocks in the assistant message batch and records synthetic error `tool_result` blocks for each.

---

**Issue 2: `execute_model_readable_error_collect` silently succeeds when tool spec cannot be found**

- **Line range**: 179-193
- **Severity**: MEDIUM
- **Description**: When `find_tool_spec(requested_tool_id).is_none()`, `find_tool_spec(&error.tool_name).is_none()`, AND `find_tool_spec(error.suggested_replacement).is_none()`, the `event_tool_id` ends up as `None`. The function falls through to `result_tool_id = requested_tool_id` (line 190-192) and still calls `model_error_to_tool_result`. However, at lines 209-222, when `event_tool_id` is `None`, the `record_tool_call_requested_preserving_provider_id` and `record_tool_call_completed_preserving_provider_id` calls are SKIPPED due to `if let Some(event_tool_id)`. This means the session never records the tool call as attempted or failed when the tool is completely unrecognized, creating an audit gap.
- **Reference**: OCC does not have this because it doesn't use `find_tool_spec` -- it has a flat tool registry.
- **Fix**: Always record the tool call attempted/completed, even when `event_tool_id` is `None`, using the `requested_tool_id` as fallback.

---

**Issue 3: Permission check loop depends on mutable `PermissionGate` reference lifetime**

- **Line range**: 714-810 (`tool_permission_decision`)
- **Severity**: MEDIUM
- **Description**: The function takes `permission_gate: &mut PermissionGate` and evaluates it, then if the result is `Ask`, it records a pending permission and returns `Pending`. The caller then needs to re-invoke this function with the same `permission_gate` after receiving the external decision. This works but is fragile: if the permission gate's internal state mutates between calls (e.g., rule updates), the second evaluation may produce a different result than the first. The TypeScript code (QueryEngine.ts line 269-271) handles this by wrapping `canUseTool` and accumulating denials in `this.permissionDenials`, keeping the gate's state independent of the permission evaluation.
- **Reference**: `QueryEngine.ts` lines 244-270
- **Fix**: Consider decoupling the "record permission state" from the "evaluate permission" steps, or snapshot the permission gate evaluation result.

---

**Issue 4: No parity with OpenClaudeCode's comprehensive error type handling**

- **Line range**: 170-265 (`execute_model_readable_error_collect`)
- **Severity**: MEDIUM
- **Description**: The TypeScript reference (`query.ts` lines 969-978) handles `ImageSizeError` and `ImageResizeError` specifically with user-friendly messages. The Rust code's `ModelReadableToolError` is a generic struct. While the Rust architecture routes tool errors differently (through `model_error_to_tool_result`), there is no equivalent image-size-specific error path.
- **Reference**: `query.ts` lines 969-978
- **Fix**: Ensure `ModelReadableToolError` can represent image size errors, or add a specific case.

---

**Issue 5: `execute_pending_tool_after_decision` calls `prepare_exact_edit_execution_args` without checking write scope**

- **Line range**: 640-653
- **Severity**: LOW
- **Description**: When executing a pending file write/edit after permission decision, the code calls `prepare_exact_edit_execution_args` to compute base hashes. However, it doesn't verify that the tool is still within its allowed write scope. The TypeScript reference tracks `allowedTools` from user input processing (QueryEngine.ts lines 412-416, 477-486).
- **Reference**: `QueryEngine.ts` lines 477-486
- **Fix**: Consider adding a post-decision write-scope check for file write/edit tools.

---

**Issue 6: `tool_args_json` duplicates logic from `permission_args_json`**

- **Line range**: 267-321 vs 963-994
- **Severity**: LOW
- **Description**: Both `tool_args_json` and `permission_args_json` serialize ParsedToolArguments into JSON strings but with slightly different logic. `tool_args_json` uses a Vec of parts joined by commas (producing a compact JSON string), while `permission_args_json` uses `serde_json::Value::Object` (producing canonical JSON). This is duplicated serialization with subtly different output.
- **Reference**: N/A (code quality)
- **Fix**: Have one function delegate to the other, or extract a shared serialization helper.

---

**Issue 7: Retry jitter uses deterministic hash rather than random**

- **Line range**: 758-764 (`native_loop_retry_delay_ms`)
- **Severity**: LOW
- **Description**: The retry delay jitter is computed from `stream_id.bytes().fold(...)` which is deterministic for a given `stream_id`. All subagents sharing the same `stream_id` format would experience the same jitter pattern. In contrast, the TypeScript retry logic (error.ts) uses `Math.random()`. While the jitter is small (mod 37), deterministic jitter can cause correlated retry storms when multiple concurrent requests share the same stream_id prefix.
- **Reference**: `query.ts` delegates to retry service; OCC's `categorizeRetryableAPIError`
- **Fix**: Use a random seed or incorporate `SystemTime` nanos into the jitter calculation.

---

### FILE 2: `native_agent_loop_model_io.rs` (1147 lines)

---

#### Bug Count by Severity

| Severity | Count |
|----------|-------|
| CRITICAL | 1 |
| HIGH    | 1 |
| MEDIUM  | 4 |
| LOW     | 2 |

#### Issues Found

---

**Issue 8: Streaming tool event handler is called before 2xx status confirmed, but visible deltas are buffered**

- **Line range**: 448-508
- **Severity**: CRITICAL
- **Description**: In the streaming observer closure (line 448), `stream_event_handler` (the tool handler) is called at line 503-508 for **every** event, **before** the 200 status is confirmed (the `live_stream_status_is_success` check at line 467 gates visible delta emission, not tool handler invocation). This means tool calls are executed (via `handle_native_stream_tool_event`) even if the HTTP response later returns a 400-and-retry. The tool calls from the failed attempt cannot be undone. In the TypeScript reference (`query.ts` lines 712-741), when `streamingFallbackOccured`, all assistant messages and tool results are cleared with tombstones, and the streaming executor is discarded. The Rust code has no equivalent undo mechanism -- tool handlers fire immediately and cannot be retracted.
- **Reference**: `query.ts` lines 712-741
- **Fix**: Defer the `stream_event_handler` invocation until after 2xx status is confirmed, OR buffer completed tool calls and only execute them after the response is verified successful. The current design is vulnerable to phantom tool executions on failed streaming attempts.

---

**Issue 9: Dual-protocol fallback on 400 does not clear stream tool handler state**

- **Line range**: 693-706
- **Severity**: HIGH
- **Description**: When a 400 response triggers protocol fallback from Anthropic to OpenAI format, the body and URL are converted and the loop continues. However, the tool handler's side effects from the failed Anthropic-stream attempt are NOT cleared. The `stream_tool_handler` closure captures mutable references to `streamed_tool_batch`, `streamed_suppressed_count`, and `observation_cache` -- tool calls already executed during the failed stream remain in these structures. On retry, duplicate tool calls may be executed, and the observation cache may contain phantom entries.
- **Reference**: `query.ts` lines 712-726 (streaming fallback clears all state)
- **Fix**: Reset `streamed_tool_batch`, `streamed_suppressed_count`, `streamed_tool_sequence`, and any observation cache mutations that occurred during the failed stream before retrying on the alternate protocol.

---

**Issue 10: Non-streaming path retry does not support error_recovery escalation on 400**

- **Line range**: 369-383
- **Severity**: MEDIUM
- **Description**: In the non-streaming path, when a 400 triggers dual-protocol fallback (lines 370-382), the `error_recovery` state is NOT updated. In contrast, the streaming path (line 693-706) also lacks this update. However, the non-streaming path at line 393-395 correctly calls `er.max_tokens.record_retry()` on transient status codes. The asymmetry means that the max_tokens escalation counter in `ErrorRecoveryState` won't trigger for 400-based protocol fallbacks, possibly causing infinite retry loops.
- **Reference**: `query.ts` lines 893-952 (FallbackTriggeredError triggers model switch, not protocol switch)
- **Fix**: Call `error_recovery.max_tokens.record_retry()` on 400 protocol fallback retries.

---

**Issue 11: `flush_live_content_stream_event` suppresses empty content, but doesn't update char/delta counters**

- **Line range**: 128-163
- **Severity**: MEDIUM
- **Description**: The function takes `pending_content: &mut String` and `pending_content_chunks: &mut usize`. If `pending_content` is empty, it returns early. But the caller (line 549-570 in the observer) checks `pending_content.is_empty()` BEFORE calling `flush_live_content_stream_event`, and only updates `live_visible_delta_count` and `live_visible_char_count` based on pre-flush state. If the content is non-empty at that point but somehow empty by the time flush runs (possible race with `std::mem::take`), the narration counts would be wrong.
- **Reference**: `query.ts` has no equivalent because it uses a different streaming architecture.
- **Fix**: Track actual flushed content length and adjust counters accordingly, or make the caller inspect the flush result.

---

**Issue 12: `compact_stream_narration` truncation uses `chars()` byte boundary vulnerability**

- **Line range**: 196-204
- **Severity**: MEDIUM
- **Description**: `content.chars().take(max_chars).collect::<String>()` operates on Unicode scalar values, but `max_chars: 4000` combined with multi-byte characters means the resulting string could be up to ~16KB in bytes. This is benign for event recording but could produce unexpectedly large event payloads.
- **Reference**: N/A (specific to Rust implementation)
- **Fix**: Consider byte-based truncation or a lower char limit.

---

**Issue 13: `guard_native_loop_prepared_request_report` leaked 'Send' variant falls through to record blocked**

- **Line range**: 855-877
- **Severity**: MEDIUM
- **Description**: The `match report.action` at line 867 only matches `CompactionRequired` and `Blocked`. The `Send` variant falls through to `"unknown"` and then records a `model_call_blocked` event even though the call was not actually blocked. This adds noise to the event log.
- **Reference**: `query.ts` line 636-648 (blocking limit check only fires when blocking)
- **Fix**: Check `report.should_send()` before entering the match block, or add an explicit `Send` arm that returns early.

---

**Issue 14: `record_native_loop_model_call_started_for_prepared_request` uses JSON-formatting of enums**

- **Line range**: 879-922
- **Severity**: LOW
- **Description**: At line 911, `format!("{:?}", context_budget.scaffold_level)` serializes the ScaffoldLevel enum variant to an event field. Using `Debug` format for event data is fragile -- renaming a variant changes the event schema.
- **Reference**: The TypeScript code uses explicit string constants (e.g., QueryEngine.ts uses `"continue"`, `"compact_boundary"`, etc.)
- **Fix**: Implement a dedicated `as_str()` or `Display` impl for `ScaffoldLevel`.

---

**Issue 15: `extract_cache_zone_hash` uses string search over full request body**

- **Line range**: 1066-1079
- **Severity**: LOW
- **Description**: The function searches the full JSON body for `<cache_zone name="X"` markers using `str::find`. This is O(n) per zone per request. Hash extraction on every model call is unnecessary overhead. The TypeScript reference (`query.ts` line 586-590) creates `dumpPromptsFetch` once per query session and wraps the fetch function.
- **Reference**: `query.ts` lines 582-590
- **Fix**: Cache the zones or extract them during request construction rather than re-parsing the serialized body.

---

### Missing Feature Count

| # | Feature | TypeScript Location | Rust Status |
|---|---------|-------------------|-------------|
| 1 | `yieldMissingToolResultBlocks` batch injection | `query.ts:123-149` | **Missing** |
| 2 | Post-sampling hooks (`executePostSamplingHooks`) | `query.ts:999-1009` | **Missing** (only per-tool hooks) |
| 3 | Structured output enforcement (`registerStructuredOutputEnforcement`) | `QueryEngine.ts:327-332` | **Missing** |
| 4 | Structured output retry limit enforcement | `QueryEngine.ts:1005-1048` | **Missing** |
| 5 | USD budget enforcement mid-turn | `QueryEngine.ts:971-1002` | **Missing** |
| 6 | Token budget continuation (TOKEN_BUDGET feature) | `query.ts:1308-1355` | **Missing** |
| 7 | Stop-failure hooks (`executeStopFailureHooks`) | `query.ts:1174, 1263` | **Missing** |
| 8 | `max_output_tokens` escalating retry (8k -> 64k) | `query.ts:1189-1221` | **Missing** |
| 9 | `max_output_tokens` recovery message injection | `query.ts:1223-1252` | **Missing** |
| 10 | Tool use summary generation (Haiku background) | `query.ts:1411-1482` | **Missing** |
| 11 | Skill discovery prefetch integration | `query.ts:326-335` | **Missing** |
| 12 | Memory prefetch (`startRelevantMemoryPrefetch`) | `query.ts:301-304` | **Missing** |
| 13 | Snip boundaries / compact message replay | `query.ts:396-410` | **Missing** |
| 14 | Image size error handling | `query.ts:969-978` | **Missing** |
| 15 | Tomstone-style message removal for streaming fallback | `query.ts:716-717` | **Missing** |
| 16 | max_turns enforcement during abort | `query.ts:1506-1513` | **Missing** |

---

### Code Quality Issues

| # | Issue | Location |
|---|-------|----------|
| 1 | Massive import block duplicated across all files in the family | All files, lines 1-87 |
| 2 | `compact_stream_narration` and related truncation functions not consolidated | `model_io.rs:196-204` |
| 3 | Raw JSON string building via format!() throughout | `model_io.rs` and `execution.rs` |
| 4 | `#![allow(unused_imports)]` at top of every file | All files, line 1 |
| 5 | Provider-family matching scattered across multiple functions | `continuation.rs:137-149` |

---

### Overall Assessment

#### `native_agent_loop_execution.rs`: Good, with gaps
The execution helper file is internally consistent and well-structured. The permission gate integration is thorough. Main concerns:
1. **Missing batch-level error injection** for tool_use blocks without results (HIGH)
2. **Edge cases in error collection** when tools are completely unrecognized (MEDIUM)
3. **Deterministic retry jitter** (LOW)

#### `native_agent_loop_model_io.rs`: Needs attention
The model I/O file has the dual-protocol fallback architecture which is well-designed, but:
1. **Streaming tool handler fires before HTTP success confirmed** (CRITICAL) -- this is the most impactful finding
2. **No state reset on protocol fallback retry** (HIGH) -- tools executed during failed streams persist
3. **16 missing features** compared to the TypeScript reference, most notably post-sampling hooks, tool use summaries, and error recovery escalation

#### Architecture Assessment
The Rust implementation takes a fundamentally different architectural approach from OpenClaudeCode. Where OCC uses a generator-based `for await` loop over yielded messages with a central `State` record holding all mutable cross-iteration data, the Rust code uses:
- Session-bound event recording (no separate yield/generator pattern)
- Callback-based stream observation with mutable references
- Protocol-level dual-fallback (Anthropic <-> OpenAI) instead of model-level fallback

This is not inherently better or worse, but it means certain OCC patterns (tombstone messages, streaming fallback undo, stop hooks) have no direct equivalent and require alternative implementations.