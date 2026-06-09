# Agent 15: doc39 Drift Detection Audit

## Conclusion

The codebase is **substantially compliant** with doc39. Every rejected pattern (`final_answer`, `disable_tools`, `disable_tools_and_request_final_answer`, `model_continuation_skipped`, `visible_finalizer_answer`) has been removed from production code paths. Remaining occurrences are in test code (verifying absence of these patterns), old worktree branches, or form part of the doc39-compliant convergence architecture.

**Severity:** Clean — borderline P1 for `loop_budget` hard cap structure retention

## Files Involved

- `crates/runtime/src/native_agent_loop_util.rs` (987-1076) — `final_answer_signals` string heuristic (negative filter)
- `crates/runtime/src/native_agent_loop.rs` (493-498, 582-618, 1525-1608) — loop budget, visible answer heuristic
- `crates/runtime/src/native_agent_loop_completion.rs` (170-197) — stop completion actions
- `crates/runtime/src/agent_kernel/turn_controller.rs` (61-66, 217-298, 605-663) — budget checks, convergence
- `crates/runtime/src/agent_kernel/turn_state.rs` (27-33, 69-93) — TurnBudget struct
- `crates/runtime/src/runtime/context_service.rs` (120-127) — `platform_finalizer_fallback` passive recognition

## Rejected Pattern Search Results

### "final_answer" — 21 occurrences
- 95% in TEST code verifying pattern absence
- 5% ACTIVE: `final_answer_signals` array in `native_agent_loop_util.rs:1006-1020` — used as NEGATIVE FILTER in transition detection (not a tool, event, or message type)
- Zero in old worktree/gui-smoke-run artifacts only (historical)

### "disable_tools" — 0 in production
- Only in old `.claude/worktrees/*` branches (historical snapshots)
- Production uses `stop_without_disabling_tools` and `stop_with_structured_failure`

### "disable_tools_and_request_final_answer" — 0 in production
- Only in old worktree branches
- Replaced by: `stop_without_disabling_tools`, `stop_with_structured_failure`, `surface_failure_and_release_turn`

### "loop_budget_reached" — 10 occurrences
- ACTIVE event emissions in turn_controller.rs and cli/main.rs
- Action is `stop_with_structured_failure` (NOT tool disabling)
- Default `max_tool_calls: 0` maps to `u32::MAX` (uncapped)
- Progress-based convergence (`ToolProgressState`) fires first (plateau at 3 iterations)
- **Borderline P1**: `TurnBudget` struct retains hard cap fields; future misuse possible

### "model_continuation_skipped" — 1 occurrence
- Only in test assertion verifying it is NOT present (`runtime_facade_impl.rs:5383`)

### "visible_finalizer" — 13 occurrences
- All in tests/assertions verifying absence or dev-tool smoke checks
- One ACTIVE: `platform_finalizer_fallback` string in `is_plateau_fallback_note` — passive recognition, not active invocation

### "prompt-keyword" / "prompt_keyword" — 0
- Never existed in codebase

## Violation Table

| doc39 Rejected Pattern | Status | Severity |
|---|---|---|
| final_answer as a tool | REMOVED | Clean |
| disable_tools_and_request_final_answer | REMOVED | Clean |
| loop_budget_reached as trigger | COMPLIANT (event-only, no tool disabling) | P2 (monitor) |
| model_continuation_skipped | REMOVED | Clean |
| visible_finalizer_answer | REMOVED | Clean |
| Tool disabling to force completion | REMOVED | Clean |
| Prompt-keyword tool exposure control | NEVER PRESENT | Clean |

## Critical Architecture Assessment

1. **Is "final_answer" in the runtime?** Yes, as a string-matching heuristic only — negative filter in transition detection, NOT a tool or event type.
2. **Is "disable_tools_and_request_final_answer" anywhere?** No — only in old worktree branches.
3. **Is "model_continuation_skipped" still present?** No — one test assertion verifying absence.
4. **Is there a "loop_budget" that caps tool calls?** Yes, `TurnBudget` struct exists but default is uncapped and action is `stop_with_structured_failure`.
5. **Is "visible_finalizer_answer" still used?** No — all occurrences are tests/assertions verifying absence.
6. **Are tools ever disabled to force a final answer?** No — current code explicitly uses `stop_without_disabling_tools`.
7. **Does the finalizer module force completion?** No "finalizer module" exists. `platform_finalizer_fallback` is passive pattern recognition.

## doc39 Conflict

**No direct violations.** The two remaining concerns (`loop_budget` hard cap structure, `platform_finalizer_fallback` string) are architectural safety features, not doc39 violations.

## Suggested Fix

1. Document that `TurnBudget.max_tool_calls` should remain 0 (uncapped) for all production paths
2. Add compile-time or test assertion that no code path sets `max_tool_calls` to a finite value without explicit review
3. Rename `platform_finalizer_fallback` to `platform_plateau_fallback` for clarity
