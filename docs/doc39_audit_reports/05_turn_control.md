# Audit 05: Turn Control + Enforcement vs doc39

**Date:** 2026-05-19 | **Files:** turn_controller.rs, evidence_ledger.rs, convergence_enforcer.rs, tool_inventory.rs, write_constraints.rs

## 1. Duplicate Code: Agent Kernel vs Native Turn Controller

**NOT duplicates.** Complementary files with different responsibilities:

- `native_turn_controller.rs`: `NativeTurnController` (tool ledger) + `evaluate_native_context_guard()` (context budget preflight) + `estimate_tokens()`
- `agent_kernel/turn_controller.rs`: Re-exports native types + defines `TurnController<Ctx>` trait + `NativeLoopTurnController` (iteration gates, progress tracking, convergence, batch guard) + all support enums

No duplicated method bodies. Dual import is intentional Phase 1 migration state.

## 2. Main Loop Import Source

`native_agent_loop.rs` imports from BOTH:
- From `crate::native_turn_controller`: `estimate_tokens`, `NativeContextGuardAction`, `NativeContextGuardReport`, `NativeTurnController` (direct)
- From `crate::agent_kernel`: `TurnController`, `NativeLoopTurnController`, `ContinuationStrategy`, `FinalizationReason`, etc. (via agent_kernel re-export)

## 3. Hardcoded turn_id Bug — CONFIRMED

`native_turn_controller.rs:53`:
```rust
Self::new_with_turn_id(format!("{session_id}:native_turn_0"))
```

`new_for_session()` tries `session.current_turn_id()` first, but falls back to the hardcoded format. If `begin_interactive_turn` wasn't called, ALL turns get the same ID. The turn index never increments.

## 4. EvidenceLedger — Fully Wired

Tracks: per-turn evidence items (provider_tool_call_id, tool_id, args, result, EvidenceClass), running counts, sealed iteration history, ContinuationView for model prompt.

Call sites: begin_iteration, record_suppressed, clear, replace from legacy batch, build continuation view, feed into ConvergenceEnforcer. **Gap:** Cloned from kernel at turn start; mutations not reflected back.

## 5. ConvergenceEnforcer — Fully Wired

Connection path:
```
NativeLoopTurnController.observe_completed_tool_iteration()
  → observe_convergence()
  → ConvergenceEnforcer.observe_iteration(evidence_ledger, batch_signature, ...)
  → ConvergenceVerdict (Continue/BatchNoveltyPlateau/DuplicateDominance/InformationStagnation/BudgetExhausted)
```

**Gap:** No convergence-to-compaction feedback. Convergence detects stagnation but doesn't trigger compaction.

## 6. ToolInventory Call Graph

| Function | Call Site |
|---|---|
| `should_finalize_tool_inventory` | native_agent_loop.rs — post-tool-batch action selection |
| `tool_inventory_summary_message` | native_agent_loop_completion.rs — builds completion message |
| `is_tool_inventory_read_only_observation` | Internal only (via observation_count) |
| `is_tool_inventory_gated_attempt` | Internal only (via gated_attempt_count) |

No truly dead functions. Two predicates are re-exported but have zero external callers.

## 7. WriteConstraints

`validate_file_write_line_count` and `requested_line_count_policy` are called from `build_fast_auto_write_request`. No longer the "only wired module" — many agent_kernel modules are now wired.

## Key Gaps

1. Hardcoded turn_id (never increments)
2. Dual import path (NativeTurnController not behind AgentKernel facade)
3. ConvergenceEnforcer ↔ Compactor disconnected
4. EvidenceLedger cloned not shared
5. Two ToolInventory predicates exported but unused externally
