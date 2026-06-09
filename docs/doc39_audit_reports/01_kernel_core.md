# Audit 01: AgentKernel Core vs doc39

**Date:** 2026-05-19 | **Files:** kernel.rs, turn_state.rs, budget_policy.rs, mod.rs

## Summary Table

| Component | Alignment | Assessment |
|---|---|---|
| AgentKernel statefulness | 100% | Holds 12 fields (not a unit struct) |
| `execute_turn()` orchestration | 0% | Missing; kernel uses `run_turn()` which delegates to old native loop |
| TurnState fields (18 spec) | 77.8% | 14 present, 4 missing |
| TurnRouter | 0% | Does not exist anywhere |
| BudgetPolicy route-awareness | 0% | Single default budget; no TurnRoute→budget mapping |
| BudgetPolicy wired into kernel | 0% | Dead code (never imported or used) |

---

## 1. AgentKernel Statefulness — PASS

`kernel.rs:177-190`: The struct holds 12 fields, NOT a `()` unit struct:

- `request_scoped`, `turn_controller`, `compactor`, `permission_gate`, `context_manager`, `evidence_ledger`, `event_log_handle`, `convergence`, `tcml`, `finalizer`, `tool_orchestration`, `profile`

**Gap:** No `BudgetPolicy` field — budget decisions are not kernel-owned.

## 2. execute_turn() Missing — FAIL

`kernel.rs:242-260`: The kernel has `run_turn()`, not `execute_turn()`. It:
1. Validates profile match
2. Delegates 100% to `run_native_agent_loop_v2_deepseek()` — a monolithic function in `native_agent_loop.rs`

The kernel owns services as fields but never activates them; the old native loop manages everything. Comment at line 171-174 acknowledges: "P3 keeps the old native loop as the execution engine for now."

## 3. TurnState Field Coverage — 77.8%

**Present (14/18):** session_id, turn_index, started_at, route, mode, role, budget, iterations, tool_calls_used, tokens_in, tokens_out, reasoning_tokens, seen_tool_batches, awaiting_user, emitted_event_count

**Missing (4/18):**
1. `reasoning_replay: ReasoningReplayState` — managed separately in native_agent_loop.rs
2. `stream_state: StreamProcessorState` — local variable, not in TurnState
3. `provider_capabilities: ToolCallingCapabilities` — never attached to TurnState
4. `last_tool_batch` — type is `Vec<String>` not `Vec<ToolBatchEntry>` (ToolBatchEntry doesn't exist)

## 4. BudgetPolicy — Dead Code

`budget_policy.rs`: Only has a single `default_budget` field with no route awareness. Never imported by kernel.rs, native_agent_loop.rs, or runtime_facade.rs. `should_compact()` duplicates `Compactor::should_compact()` but is only called from its own tests.

## 5. No TurnRouter Exists — FAIL

`grep -rn "TurnRouter"` returns zero results. `TurnRoute` enum (9 variants) exists at `turn_state.rs:6-15` but is never used in routing decisions. It's purely decorative.

## 6. Public API Surface

**kernel.rs public functions:** ContextManager::guard_prepared_request, TcmlService::process_text, TcmlService::extract_final_answer_tool_call, Finalizer methods (3), ToolOrchestrationService methods (3), AgentKernel::for_request, AgentKernel::run_turn

**Imported by native_agent_loop.rs:** 18 symbols
**Imported by runtime_facade.rs:** 3 symbols

## Recommendations

1. Rename `run_turn` → `execute_turn` and implement orchestration loop inside kernel
2. Build a `TurnRouter` that selects TurnRoute based on request content
3. Add 4 missing TurnState fields
4. Wire BudgetPolicy into kernel or delete it (currently dead)
5. Wire Compactor into the turn loop
6. Delete or consolidate duplicate `should_compact()` methods
