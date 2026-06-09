# Agent 8: Context / Compaction Audit

## Conclusion

Compaction threshold is correctly DeepSeek-aware (192K). However, compaction is entirely in-process (no separate Compactor/Flash model role), irreversible (lossy text summary, not reconstructable), and reasoning_content is reduced to a 240-char preview that can't support actual replay. There is no L1 state object — all state is carried as raw conversation or markdown summary text.

**Severity:** P2 (in-process compaction is a doc39 deviation; reasoning fold breaks replay)

## Files Involved

- `crates/runtime/src/agent_kernel/compactor.rs` (29-205) — in-process Compactor struct
- `crates/runtime/src/agent_kernel/observation_cache.rs` — duplicate detection
- `crates/runtime/src/agent_kernel/evidence_ledger.rs` (141-434) — evidence management
- `crates/runtime/src/native_turn_controller.rs` (290-425) — `evaluate_native_context_guard`
- `crates/runtime/src/context_budget.rs` (11-18) — `DEEPSEEK_COMPACTION_THRESHOLD_TOKENS`
- `crates/runtime/src/context_policy.rs` (26-47) — native context policies
- `crates/runtime/src/compaction.rs` (5-40) — `CompactionSummary`
- `crates/runtime/src/native_profile/deepseek/reasoning.rs` (149-234) — `ReasoningReplayManager`
- `crates/runtime/src/native_agent_loop_continuation.rs` (152-175, 246-264)
- `crates/runtime/src/event_log.rs` (115-139) — `project_context`

## Key Findings

### Finding 1: Compaction Is In-Process, No Flash Model (P2 — Major doc39 deviation)

The `Compactor` struct has no model adapter, no HTTP client, no endpoint. The `compact` method takes an `&EventLog` and returns a `CompactionResult` struct — pure computation. The `compactor_role_calls` telemetry counter is incremented by `context.compaction.projected` events, NOT by actual model calls. **doc39 §12 specifies a separate Flash model for compaction.**

### Finding 2: No L1 State Object (P1)

`CompactionSummary` is a markdown blob with `Vec<String>` fields. No serializable state object exists that could reconstruct the original context. The `project_context` method produces flat strings from events.

### Finding 3: Compaction Is Irreversible (P1)

Original events before the preservation boundary are not included in the projection sent to the model. Raw EventLog is preserved on the runtime side but the model cannot "see" old events.

### Finding 4: reasoning_content Reduced to 240-char Preview (P2)

`latest_reasoning_preview` scans EventLog backwards and extracts at most 240 chars. After `compact_old_reasoning` replaces raw reasoning with `"[reasoning folded at turn N]"`, the next model call gets no useful reasoning signal. **doc39 requires preserving latest reasoning for replay.**

### Finding 5: Threshold Correctly DeepSeek-Aware (P0 ✓)

`min(192000, context_window * 3/4)` correctly implemented in `deepseek_compaction_threshold_tokens`. For DeepSeek with 256K limit: `min(192000, 192000) = 192K`.

### Finding 6: "below_threshold" Telemetry Noise (P3)

Every model call under 192K emits `context.compaction.skipped` with `reason=below_threshold`, including trivially small requests.

### Finding 7: Token Estimation Is Naive (P2)

`estimate_tokens` uses `chars/4` vs `word_count`, taking the max. For structured JSON with many short tokens, this systematically underestimates actual token counts.

### Finding 8: Duplicate Observation Detection Works Within-Turn Only (P3)

ObservationCache prevents re-sending the same tool call within an iteration but does not persist across turns. The dedup is purely in-memory.

## doc39 Conflict Summary

| § | Requirement | Status |
|---|---|---|
| §12 | Separate Flash model for compaction | **Conflict** — in-process only |
| §6 | Preserve latest reasoning for replay | **Conflict** — 240-char preview only |
| §4 | L1 state object | **Conflict** — not implemented |
| §5 | Reversible compaction | **Partial** — EventLog preserved but model can't see old events |
| §13 | 192K threshold | **Compliant** |

## Suggested Fix

1. Add optional LLM-based compaction using Flash model role (or document decision to stay in-process)
2. Implement L1 state object for compact state representation
3. Preserve full reasoning_content in ReasoningReplayManager through compaction
4. Improve token estimation with proper tokenizer

## Handoff Needed

- Agent 5 (DeepSeek Stream) — reasoning_content preservation during compaction
- Agent 1 (Agent Loop) — compaction trigger integration in main loop
