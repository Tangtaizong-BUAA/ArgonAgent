# Agent 2: Permission Resume Audit

## Conclusion

Plan approval, file write approval, and shell approval use **different resume models**. Plan approval records a synthetic artifact and starts a new prompt round. Tool permissions (file write, shell) re-execute the original tool, then continue the model loop. There is no exactly-once guard on the native RuntimeFacade resume path, a race condition between cancel and tool re-execution, and `pending_native_decision` is not cleared on session cancel.

**Severity:** P1 (race condition + missing exactly-once guard)

## Files Involved

- `crates/runtime/src/runtime_facade_impl.rs` — lines 365-597 (tool permission resume), 599-684 (plan decision), 54-79 (cancel_session), 3948-4018 (identity inference)
- `crates/runtime/src/session.rs` — lines 324-403 (state transitions for both paths)
- `crates/runtime/src/local_api_server.rs` — lines 1111-1282 (HTTP dispatch), 1293-1326 (polling loop)
- `desktop/src-tauri/src/main.rs` — lines 400-558 (Tauri command dispatch), 875-995 (polling + continue spawn)
- `crates/runtime/src/native_agent_loop_resume.rs` — lines 98-190 (legacy resume path with exactly-once guard)

## Events Involved

- `permission.requested`
- `permission.decided`
- `runtime.permission_resume.started`
- `runtime.permission_resume.completed`
- `runtime.permission_resume.tool_executed`
- `tool.call_completed`
- `tool.result_recorded`

## State Involved

- `WaitingForToolApproval`
- `WaitingForPlanApproval`
- `ApplyingPatch`
- `RunningCommand`
- `Executing`
- `DiagnosingFailure`

## Key Findings

### 1. Different Resume Models

| Property | Plan Approval | Tool Permission |
|---|---|---|
| Tool re-execution | No (synthetic artifact) | Yes (execute_tool) |
| Model continuation | Always on approve | Only if result.ok |
| State transition | → RetrievingContext | → Executing / DiagnosingFailure |
| Caller action | New prompt with continue text | New loop with same prompt |
| Exactly-once guard | N/A | No (legacy path has it, native doesn't) |

### 2. Race Condition (P1)

In `resume_native_loop_after_permission_decision`:
1. Session locked, permission validated, `session.decide_permission()` called
2. Lock dropped
3. **UNLOCKED WINDOW**: `execute_tool()` runs without checking interrupt flag
4. Lock re-acquired, tool result recorded on potentially cancelled session

`cancel_session()` does not clear `pending_native_decision`.

### 3. No Exactly-Once Guard on Native Path (P1)

The legacy `resume_native_agent_loop_after_external_decision` checks `replayed_tool_completion_state`. The newer native RuntimeFacade path does NOT. Double-submission (e.g., GUI double-click) would execute the tool twice.

### 4. Identity Preservation

Pending tool identity is preserved via `RuntimePendingNativeDecision`:
- `permission_id` — unique
- `tool_call_id` — reconstructed from event log
- `provider_tool_call_id` — extracted from event log
- `args` — stored for re-execution

## doc39 Conflict

**Yes.** `session.decide_permission` routes FileWrite → `ApplyingPatch` and Command → `RunningCommand`. doc39 specifies all approved tool requests should go through a unified permission evaluation → dispatch path, not distinct per-tool-type sub-states.

## Suggested Fix

- Add interrupt check before `execute_tool()` at line 460
- Clear `pending_native_decision` in `cancel_session`
- Add exactly-once guard to native RuntimeFacade resume path
- Implement idempotency check on `permission_id`

## Not Suggested

- Do NOT unify plan approval and tool approval resume models — they serve different purposes
- Do NOT add a global lock across tool execution — slows the fast path

## Handoff Needed

- Agent 10 (Active Turn / Cancel) — coordinate on interrupt/cancel race
- Agent 14 (Security / Shell Classifier) — coordinate on permission gate integration
