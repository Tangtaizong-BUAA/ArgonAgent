# Agent 1: Agent Loop Main Cycle Audit

## Conclusion

The agent loop has a single `for` loop bounded by `max_iterations` (default 8), but with 23 distinct exit paths across preflight checks, HTTP errors, content analysis, tool gating, and budget exhaustion. There are 6 distinct "loop owners" that each can decide to stop. The tool-result-to-model path is always followed (no skip paths found). The terms `disable_tools`, `model_continuation_skipped`, and `visible_finalizer_answer` are NOT in active code paths. A visible-answer heuristic serves as an implicit finalizer.

**Severity:** P1 (6 loop owners create fragmented stop authority; EscalateToCodeEdit changes manifest mid-loop)

## Files Involved

- `crates/runtime/src/native_agent_loop.rs` (2924 lines) — main loop orchestrator
- `crates/runtime/src/agent_kernel/turn_controller.rs` (2080 lines) — iteration policy, convergence, batch guard
- `crates/runtime/src/agent_kernel/turn_state.rs` (468 lines) — TurnState, ToolProgressState, TurnBudget
- `crates/runtime/src/agent_kernel/kernel.rs` (348 lines) — AgentKernel service facade
- `crates/runtime/src/native_turn_controller.rs` (633 lines) — context guard, tool ledger
- `crates/runtime/src/native_agent_loop_completion.rs` (256 lines)
- `crates/runtime/src/native_agent_loop_continuation.rs` (491 lines)
- `crates/runtime/src/native_agent_loop_execution.rs` (1025 lines)
- `crates/runtime/src/native_agent_loop_model_io.rs` (1147 lines)
- `crates/runtime/src/native_agent_loop_tools.rs` (1856 lines)
- `crates/runtime/src/native_agent_loop_prompt.rs` (713 lines)

## 23 Exit Paths (Complete)

### A. Preflight (before model call)
1. **Interrupted** — `interrupt` AtomicBool set → `Interrupted`
2. **ToolLimitFailed** — `tool_call_count >= max_tool_calls && !has_last_tool_batch` → `Failed`
3. **ToolLimitStop** — `tool_call_count >= max_tool_calls && has_last_tool_batch` → `Blocked` (emits `agent.loop_budget_reached`)

### B. Transport/HTTP
4. **Compaction blocked** — compaction required but no valid summary → `Blocked`
5. **Guard blocked (pre-send)** — context guard returns `should_send == false` → `Blocked`
6. **Compacted guard blocked** — after compaction, still blocked → `Blocked`
7. **Tool_result continuation HTTP failure** → `Failed`
8. **Plain evidence HTTP failure** → `Failed`
9. **Initial model HTTP failure** → `Failed` via `stop_with_structured_failure`
10-11. **Transport errors** (file gen / non-file-gen) → `Failed` or `Err`

### C. Content-based
12. **Output truncation recovery** — `stop_reason == length` → continue with escalated budget
13. **Visible final answer** — no tool calls, non-transition visible text → `Completed`
14. **Visible answer rejected** — text empty or budget refusal → `Failed`
15. **Empty visible response** — no tools, empty content → `Failed`

### D. Tool-related
16. **Batch guard stop** — repeated batch exhaustion → `Blocked`
17. **plan.enter** — → `Blocked`
18. **ask_user** — → `Blocked`
19. **Write tool pending permission** — → `Blocked` with `pending_tool`
20. **Shell command pending permission** — → `Blocked` with `pending_tool`
21. **Tool control stop** — convergence/progress stop → `Blocked`

### E. Loop exhaustion
22. **Loop ended with batch** — natural completion, has pending batch → `Blocked`
23. **Loop ended without batch** — natural completion, no batch → `Failed`

## Implicit Finalizer

The visible answer heuristic (lines 1529-1597): when model produces text with no tool calls AND the text doesn't match transition patterns (≤80 chars, preamble phrases like "I'll", "Let me"), the loop infers completion. No explicit model signal required.

## 6 Loop Owners

| # | Owner | What It Controls |
|---|---|---|
| 1 | NativeLoopTurnController | Preflight checks (interrupt, tool limit) |
| 2 | Main loop body | HTTP status, streaming, visible content, tool existence |
| 3 | ToolOrchestrationService | Repeated/alternating batch patterns |
| 4 | NativeLoopTurnController | Progress aggregation, convergence |
| 5 | ConvergenceEnforcer | Duplicate/stagnation/plateau decisions |
| 6 | ToolProgressState | Duplicate/non-progress/error plateau thresholds |

## Search Term Results

| Term | Found? | Notes |
|---|---|---|
| `final_answer` | YES | Only in `visible_text_looks_like_transition_statement` — negative signal |
| `disable_tools` | NOT FOUND | Not in any audited file |
| `loop_budget_reached` | YES | Event type in `begin_iteration()` and loop exhaustion |
| `model_continuation_skipped` | NOT FOUND | Not present |
| `visible_finalizer_answer` | NOT FOUND | Not present |

## Key Positive Finding

After a tool result is recorded, the loop **always** goes back to the model (next iteration). No skip paths exist.

## doc39 Conflict

**Partial.** The `loop_budget_reached` event still exists but only fires at max_iterations exhaustion (budget guard, not tool cap). `max_tool_calls` defaults to 0 (uncapped). The EscalateToCodeEdit mid-loop manifest change is the main concern.

## Suggested Fix

Consolidate loop ownership — NativeLoopTurnController should be the single stop authority. Remove EscalateToCodeEdit or make it a new turn rather than mid-loop manifest change.

## Handoff Needed

- Agent 7 (Tool Policy) — EscalateToCodeEdit manifest change
- Agent 9 (Long Task Progression) — plateau/convergence mechanisms
