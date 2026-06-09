Now I have all three files fully read. Here is the comprehensive line-by-line review.

---

## File 1: `native_agent_loop.rs` (2923 lines)

### Overall Comparison with TypeScript Reference

The Rust main loop function `run_native_agent_loop_v2_deepseek_inner` (line 411-2919) corresponds to `queryLoop` in `query.ts` (line 241-1729). The TS version is a `while(true)` loop with state reassignment at 7 named continue sites, each with a typed `.transition.reason`. The Rust version is a `for iteration in 0..max_iterations` loop with mutable variable mutation and no explicit transition tracking.

---

### CRITICAL Issues

#### CRITICAL-1: Lines 718-725, 776-782, 812-819, 1181-1188, 1227-1234, 1264-1271 -- Model call count inflated when no model was called

When `NativeContextGuardAction::CompactionRequired` fires but the guard report has no `compaction_summary`, or when the guard says `should_send() == false`, the code increments `model_call_count` and returns `Blocked`/`Failed`:

```rust
// Lines 716-725 (continuation path)
if guard_report.action == NativeContextGuardAction::CompactionRequired {
    let Some(summary) = guard_report.compaction_summary.as_ref() else {
        model_call_count += 1;  // ←BUG: no model was called here
        return Ok(loop_result(...));
    };
```

These are PRE-call guard checks. The model HTTP round-trip has not happened yet. The guard is checking token estimates on the prepared request BEFORE sending. Incrementing the counter here makes `model_call_count` an unreliable metric. Same bug appears at 6 sites: lines 718-725, 776-782, 812-819 (continuation path) and lines 1181-1188, 1227-1234, 1264-1271 (initial path).

**TS reference**: `turnCount` is only incremented at continue sites that have completed a full model+tool loop iteration (line 1679: `const nextTurnCount = turnCount + 1`). The TS never increments a counter for a path that didn't call the model.

**Fix**: Remove `model_call_count += 1` from all 6 pre-call guard failure sites.

---

#### CRITICAL-2: Lines 260-278, 360-376, and throughout -- JSON injection via `format!()` string templating

The `record_context_compaction_completed_after_guard` function (line 244) and `record_output_truncation_recovery_event` (line 350) build JSON payloads with `format!()` using `json_string()`:

```rust
format!(
    "{{\"call_id\":{},\"retry_call_id\":{},\"stage\":{},...}}",
    json_string(&guard_report.call_id),
    ...
)
```

`json_string()` wraps the value in quotes. If `summary.to_markdown()` (line 275) contains unescaped double quotes or newlines (which markdown commonly does), the resulting JSON will be structurally invalid. This is a real injection risk for any value that comes from model output or file content.

**TS reference**: The TS uses typed objects and `JSON.stringify()` implicitly through message creation functions, never building JSON with string interpolation.

**Fix**: Use `serde_json::json!()` or equivalent for all event payloads. Replace all `format!`-based JSON construction with structured serialization.

---

#### CRITICAL-3: Line 556 -- Loop bound evaluated once at entry, doesn't respond to mid-loop budget changes

```rust
for iteration in 0..turn_state.budget.max_iterations as usize {
```

This evaluates `turn_state.budget.max_iterations` once when the `for` begins. If `max_iterations` changes during the loop (e.g., escalation), the loop does not respect the new bound.

**TS reference**: Uses `while (true)` (line 307) and checks `maxTurns` at the *end* of each iteration (line 1705: `if (maxTurns && nextTurnCount > maxTurns)`), not the beginning. This means the check is always fresh.

**Fix**: Convert to a `while` loop that checks `iteration_index < turn_state.budget.max_iterations as usize` each iteration, or add a break check at iteration end (mirroring TS line 1705).

---

### HIGH Issues

#### HIGH-1: Lines 586-593 — `reason` from `IterationOutcome::Block` silently discarded

```rust
IterationOutcome::Block { reason } => {
    if let Some(error) = iteration_error {
        return Err(error);
    }
    let _ = reason;  // ← dead binding, reason is discarded
    return Ok(loop_result(
        NativeAgentLoopStatus::Failed,  // ← "Blocked" downgraded to "Failed" without reason
        ...
    ));
}
```

The `reason` string from the iteration controller is neither logged nor returned. The caller gets `Failed` with no explanation of WHY iteration was blocked.

**TS reference**: TS doesn't have a direct `Block` outcome. Instead it exits with explicit typed terminal reasons: `{ reason: 'blocking_limit' }` (line 646), `{ reason: 'max_turns' }` (line 1711), etc.

**Fix**: Either log the reason as a runtime event before returning, or change `loop_result` to include a failure reason.

---

#### HIGH-2: Lines 627-1079 vs 1080-1401 — Massive duplicated structure between continuation and initial request paths

The `!last_tool_batch.is_empty()` branch (continuation request, ~450 lines) and `else` branch (initial request, ~320 lines) share the same structure:
1. Build messages / prepare request
2. Guard for compaction
3. Rebuild compacted request if needed
4. Send via transport
5. Record response
6. Check for pending tools / HTTP errors

The two branches encode the same logic with minor parameter differences. This makes bugs likely to be fixed in one path but not the other (C1 above is already an example -- the model_call_count bug exists in both paths).

**TS reference**: TS has a single code path (lines 654-708) that calls `deps.callModel(...)` with the same parameter structure regardless of whether it is the first turn or a continuation.

**Fix**: Extract the shared model-call-with-compaction-guard logic into a single function parameterized by the call context (continuation vs initial).

---

#### HIGH-3: Lines 1704-1707 — `last_tool_batch.clear()` and `evidence_ledger.clear()` before concurrent execution section

```rust
last_tool_batch.clear();       // line 1705
evidence_ledger.clear();       // line 1706
iteration_controller.reset_loop_guard_recovery(); // line 1707
```

This is placed correctly (before both concurrent and sequential execution sections), but the concurrent execution section (line 1718-1861) writes to `last_tool_batch` and `evidence_ledger`, and the sequential section (lines 1864-2776) writes to them afterward. If a concurrent execution batch succeeds but the program exits early (e.g., due to a permission block on a subsequent sequential tool), the evidence from completed concurrent tools has already been appended. This is arguably correct behavior (completed tools should be remembered), but it differs from the TS pattern where all tool results are collected atomically in one array and only committed on the next `state = next` assignment.

**TS reference**: TS builds `toolResults` as a local array (line 552), appends results one at a time (line 1395), then commits the entire `[...messagesForQuery, ...assistantMessages, ...toolResults]` array in a single state reassignment at the continue site (line 1716). Partial tool results from an aborted iteration never contaminate state.

**Fix**: Collect all tool results for an iteration into a local staging vector, and only commit to `last_tool_batch`/`evidence_ledger` when the entire iteration completes without early-return.

---

#### HIGH-4: Lines 1714-2801 — `distinct_keys_before` captured ~1100 lines before use

```rust
let distinct_keys_before = turn_state.observation_cache.distinct_key_count(); // line 1714
// ... tool execution, observation cache mutations, etc. ...
ToolIterationControlInput {
    distinct_keys_before,  // line 2801
    ...
}
```

The value is captured at line 1714 but the concurrent execution path (lines 1843-1847) and the sequential execution path (lines 2605-2607) both call `check_and_record_in_workspace` which mutates the observation cache. Since `distinct_keys_before` is an immutable snapshot, this is technically correct -- the value captured at line 1714 correctly represents the state before tool execution. However, the 1100-line gap between capture and use makes this dependency fragile: any future refactor that adds cache mutations between line 1714 and the tool execution entry could break the semantics.

**No TS equivalent**: TS doesn't track distinct observation key growth this way.

**Fix**: Move the capture to immediately before the first observation cache mutation, inside a small scope, and document the invariant.

---

### MEDIUM Issues

#### MEDIUM-1: Lines 529-534 — Unnamed tuple type for `last_tool_batch`

```rust
let mut last_tool_batch: Vec<(
    String,  // provider_tool_call_id
    String,  // tool_id
    String,  // arguments_json
    crate::tool_execution::ToolExecutionResult,  // result
)> = Vec::new();
```

Four unnamed fields in a tuple that appears in ~40 locations. The comment at lines 524-528 even has to explain what the fields are. This is highly error-prone -- transposing two String fields compiles silently.

**Fix**: Define a named struct with 4 fields.

---

#### MEDIUM-2: Lines 97-139 and sub-module files -- All 12 sub-modules have identical 87-line import blocks

Every `#[path]` sub-module (`native_agent_loop_completion.rs`, `native_agent_loop_continuation.rs`, etc.) begins with an identical `use crate::...` block ~87 lines long. This is a maintenance hazard: adding or removing a dependency in the parent module requires touching all sub-modules.

**Fix**: These sub-modules are `mod` declarations with `#[path]` in the parent. They don't need their own `use` blocks at all if they import via the parent module's namespace. Use `use super::*` or `use crate::native_agent_loop::*` in sub-modules.

---

#### MEDIUM-3: Lines 556-558 — `let (iteration_outcome, iteration_error)` destructing with temporary borrow

```rust
let (iteration_outcome, iteration_error) = {
    let mut iteration_context = NativeLoopIterationContext {
        session: &mut session,  // borrows session
        ...
    };
    let outcome = iteration_controller.run_iteration(&mut iteration_context);
    (outcome, iteration_context.error.take())
}; // session borrow released here
```

This is correct (the borrow is released at the block boundary), but the comment at line 523-528 about `last_tool_batch` being "backward-compatibility batch" suggests technical debt. The comment itself is 6 lines explaining that novelty/convergence must not be derived from this vector and that ObservationCache should be used instead. This indicates `last_tool_batch` should be extracted into a dedicated type with documented invariants.

---

#### MEDIUM-4: Lines 1484-1487 — `u32` to `usize` loop conversion

```rust
for _ in 0..streamed_suppressed_count {
    evidence_ledger.record_suppressed();
}
```

`streamed_suppressed_count` is `u32` (line 624). The range `0..streamed_suppressed_count` on a 64-bit platform would implicitly promote to `u32` range. This works but is fragile -- if `streamed_suppressed_count` were changed to `u64`, this would be a compile error. Same at lines 1711-1713.

**Fix**: Add `.into()` or `as usize` for explicitness.

---

#### MEDIUM-5: Lines 1536-1558 — Transition statement guard is novel (no TS equivalent), but incomplete

The "preamble-only" guard checks if the model emitted text that looks like a transition statement ("好的，让我查看...") instead of a real tool call or answer. This is a valuable guard but:

1. It only works after prior tool work (`prior_tool_work` check at line 1544). A transition statement on the first iteration (with no prior tools) passes through as though it were a real answer.
2. The detection function `visible_text_looks_like_transition_statement` is not visible in this file, so its robustness cannot be assessed here.

**TS reference**: TS doesn't have this guard. The TS model tends to produce fewer preamble-only outputs because Anthropic models are trained differently. This is a provider-specific mitigation.

---

#### MEDIUM-6: Lines 244-278 -- `record_context_compaction_completed_after_guard` takes but ignores `compacted_stage` semantic value

The `compacted_stage` parameter is used only in the JSON payload string. If the caller passes a wrong value, there is no type-level enforcement. The field is used at only 2 call sites (line 702 and 1165) with values `"tool_continuation"` and `"initial"` (and compacted variants `"compacted_tool_continuation"` and `"compacted_initial"`).

**Fix**: Use an enum instead of `&str` for `compacted_stage`.

---

### LOW Issues

#### LOW-1: Line 1 (and all sub-modules) -- `#![allow(unused_imports)]`

Inner attribute allowing unused imports across all import blocks. This suppresses real warnings that could catch dead dependencies.

**Fix**: Remove the attribute and clean up unused imports.

---

#### LOW-2: Line 141 -- Magic string constant

```rust
const EXTERNAL_PERMISSION_NOT_ALLOWED: &str = "__native_loop_external_permission_not_allowed__";
```

This constant is defined but searching for usage: it appears only in this definition. It may be used in sub-modules but if not, it is dead code.

---

#### LOW-3: Lines 555 (per_iteration_tool_cap) vs 254 of TS

```rust
let per_iteration_tool_cap = 8usize;
```

TS has no per-iteration tool cap. Tools from a single model response are all executed. This cap silently drops tool calls beyond the 8th from a single model response. If the model emits 10 tool calls, the last 2 are silently ignored with no error event.

**Fix**: At minimum, record an event when tool calls beyond the cap are dropped.

---

#### LOW-4: Lines 822 -- Variable shadowing (call_id, stream_id, transcript_id)

```rust
let call_id = active_call_id;       // line 820
let stream_id = active_stream_id;   // line 821
let transcript_id = active_transcript_id; // line 822
```

These shadow the outer `call_id`, `stream_id`, `transcript_id` from line 620-622. After line 822, the original values are inaccessible. This is intentional (the compaction path may have changed them), but shadowing makes debugging harder.

**Fix**: Use `let (call_id, stream_id, transcript_id) = (active_call_id, active_stream_id, active_transcript_id)` to make it clear this is a forced reassignment.

---

#### LOW-5: Lines 2921-2923 -- Test module at bottom

```rust
#[cfg(test)]
#[path = "native_agent_loop_tests.rs"]
mod native_agent_loop_tests;
```

Tests are in a separate file. This is fine, but having the tests visible briefly is good practice. No issue beyond noting.

---

### Missing Features (compared to TS reference)

| # | TS Feature | TS Lines | Status in Rust |
|---|-----------|----------|----------------|
| 1 | Token budget with auto-continue | 1308-1355 | **Missing** |
| 2 | `ToolUseSummary` generation (Haiku) | 1411-1482 | **Missing** |
| 3 | Memory prefetch (`startRelevantMemoryPrefetch`) | 301-304, 1599-1613 | **Missing** |
| 4 | Skill discovery prefetch | 331-335, 1619-1628 | **Missing** |
| 5 | Stop hooks (`handleStopHooks`) | 1267-1276 | **Missing** |
| 6 | Post-sampling hooks | 1000-1009 | **Missing** |
| 7 | Model fallback (`FallbackTriggeredError`) | 893-953 | **Missing** |
| 8 | Image size/resize error handling | 969-978 | **Missing** |
| 9 | Queued command draining | 1570-1643 | **Missing** |
| 10 | Cached microcompact boundary messages | 870-892 | **Missing** |
| 11 | `maxTurns` per-turn check | 1705-1712 | **Incorrectly positioned** (see C3) |
| 12 | Snip compact integration | 401-410 | **Missing** |
| 13 | Context collapse integration | 440-447 | **Missing** |
| 14 | Tool result budget enforcement | 373-394 | **Missing** |

Note: Most of these are intentionally deferred (Phase 2+). Items 1, 5, 6, 7, 8 are the most impactful gaps for production readiness.

---

## File 2: `native_agent_loop_completion.rs` (256 lines)

### Issues

#### COMPLETION-1 (MEDIUM): Lines 102-158 -- `record_native_loop_turn_summary` builds JSON via `format!()`

Same JSON injection concern as in the main file. The `safe_json_fragment()` wrapper is used for the array parts but the overall structure is still `format!`-based.

---

#### COMPLETION-2 (LOW): Lines 126, 153 -- `let _ = session.record_runtime_event(...)` discards errors

```rust
let _ = session.record_runtime_event(...);
```

If recording the event fails, the error is silently discarded. This means a failed telemetry write doesn't propagate, which is acceptable for non-critical events, but should be documented.

---

#### COMPLETION-3 (LOW): Line 89 -- Unnecessary repetition of `use crate::native_agent_loop::` imports

The sub-module imports from parent paths explicitly rather than using `use super::*`. This is the same issue as MEDIUM-2 above.

---

#### COMPLETION-4 (OK): Lines 199-256 -- `record_visible_assistant_message` is sound

This function correctly checks for empty/refusal text, records block lifecycle events, and returns `Ok(false)` for invalid content. This matches the TS pattern of filtering bogus assistant outputs before committing them.

---

#### COMPLETION-5 (LOW): Line 204 -- `visible_text_without_tool_calls` strips DSML markup but is called here

This is a re-parse of content that in the main loop has already had tool calls extracted (line 1430-1436). The redundant parse is cheap (string scanning) and provides safety against calling this function with content that hasn't been pre-parsed, so this is acceptable.

---

## Summary

### Bug Count by Severity

| Severity | Count | Description |
|----------|-------|-------------|
| CRITICAL | 3 | Model call count inflation (6 sites), JSON injection, loop bound stale |
| HIGH | 4 | Block reason discarded, duplicated model call paths, mutable state commit timing, distinct_keys_before distance |
| MEDIUM | 6 | Unnamed tuple, redundant imports, u32/usize conversion, weak type for compacted_stage, transition guard incomplete, unused constant |
| LOW | 5 | unused_imports suppress, magic string, variable shadowing, discarded errors, tool cap silently drops calls |

### Missing Feature Count: 14 (see table above)

### Code Quality Issues Count: 3
1. Manual JSON construction instead of serde (systemic, affects ~30+ sites)
2. 2300-line loop body in `run_native_agent_loop_v2_deepseek_inner`
3. Sub-module files have identical 87-line import blocks (12 files)

### Overall Assessment

**Not yet production-ready.** The three critical bugs (model call counter inflation, JSON injection risk from string-constructed payloads, and the stale loop bound) should be fixed before any production deployment. The 4 high-severity issues represent architectural debt that will cause reliability problems under load (particularly the duplicated continuation/initial code paths and the mutable state commit timing). Of the 14 missing TS features, the model fallback, image error handling, and stop hooks are the most important for production robustness.

The core agent loop logic (model call, tool parsing, tool execution, result collection, re-prompting) is functionally correct and matches the TS reference's intent. The main divergence is in code organization (inline sequential tool execution vs structured executor pattern) and compaction strategy (reactive guard vs proactive pre-check).
</parameter>