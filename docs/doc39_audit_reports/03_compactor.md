# Audit 03: Compactor vs doc39

**Date:** 2026-05-19 | **Files:** compactor.rs, convergence_enforcer.rs, native_turn_controller.rs

## Summary

| Requirement | Status | Detail |
|---|---|---|
| Triggers at 192K (B13) | PASS | `DEEPSEEK_COMPACTION_THRESHOLD_TOKENS = 192_000` |
| Preserves last 4 turns | PASS | `preserve_recent_turns: 4` default |
| Preserves latest reasoning (B1,B2) | PARTIAL | Text preview in summary; raw in ReasoningReplayManager |
| Calls compact_old_reasoning() | PASS | Called at both call sites |
| ACTUALLY MODIFIES EventLog | FAIL | `compact()` is read-only (&EventLog) |
| preserve_latest_reasoning used | PASS | Checked at compactor.rs:162 |
| Called before model requests | PASS | Via guard chain at two call sites |

---

## 1. Compaction Trigger Chain (Full)

```
Step 1: Request built (initial or continuation)
Step 2: guard_native_loop_prepared_request_report()
  → context_manager.guard_prepared_request()
  → evaluate_native_context_guard()    (native_turn_controller.rs:290)
Step 3: Token estimation
  estimated_total = estimate_tokens(body) + max_tokens
  target_limit = 240K (DeepSeek)
  hard_limit = 256K
  compaction_threshold = 192K
Step 4a (BLOCK): total > 240K → emit context.compaction.blocked → abort
Step 4b (COMPACT): total > 192K → compactor.compact() → emit completed → CompactionRequired
Step 5: Loop rebuilds request with compaction summary, re-guards, sends
```

## 2. Read vs Write Analysis

**`compact()` is strictly read-only.** Takes `&EventLog` (immutable ref). Never calls `.append()` or any mutable method. Returns an in-memory `CompactionResult`.

**The actual compaction happens at request-build time:** old turns are omitted from the rebuilt HTTP request, replaced by markdown summary. The EventLog remains append-only and intact. This is architecturally defensible (no data loss) but diverges from doc39 §12 which says Compactor "ACTUALLY MODIFIES the EventLog."

## 3. Reasoning Replay Connection

Connected at the call site (not inside Compactor):
1. `evaluate_native_context_guard()` → `Compactor::compact()` → summary
2. Back in loop: `reasoning_replay.compact_old_reasoning()` → folds raw reasoning

After compaction, raw reasoning for B1 replay is replaced by placeholder `"[reasoning folded at turn N -- ...]"` — so B1 cross-turn reasoning replay breaks after compaction.

## 4. ConvergenceEnforcer — Fully Wired

Despite 143 lines, `ConvergenceEnforcer` IS wired:
- Held on `AgentKernel` struct (kernel.rs:185)
- Called via `NativeLoopTurnController.observe_convergence()` → `observe_iteration()`
- Verdicts: Continue / BatchNoveltyPlateau / DuplicateDominance / InformationStagnation / BudgetExhausted
- Maps to: Continue / SoftWarning / Escalate / Finalize

**Gap:** No convergence-to-compaction feedback. Convergence detects "information stagnation" but doesn't trigger compaction.

## 5. Key Architectural Gap

Compaction is entirely read-only on EventLog but mutating on HTTP request body. The EventLog grows unboundedly across turns while the model sees a compacted view. Per doc39 §12, the spec says Compactor should modify EventLog.
