# ADR: Sync Runtime With Bounded Blocking Before Tokio Migration

Status: Accepted for the current architecture-upgrade slice.

Date: 2026-05-10

## Context

`docs/engineering/architecture_upgrade_from_claude_code.md` identified real
blocking-boundary risk in the agent runtime. The unsafe interpretation would be
to make a full `tokio` migration the first repair step. That would add new
dependencies, churn a large test surface, and blur the user-visible approval and
tool-resume bugs with a runtime rewrite.

The current repo already has a synchronous runtime with:

- append-only event/session replay;
- explicit shell timeout support;
- Python sidecar/provider boundaries with request-level HTTP timeouts;
- injectable transport tests for provider behavior;
- GUI/Tauri command boundaries over `RuntimeFacade`.

## Decision

Keep the runtime synchronous for this slice, but enforce bounded blocking and
lock discipline:

- no tool execution while holding the global `RuntimeFacade.sessions` mutex;
- no permission-resume tool execution while holding the global session lock;
- provider/tool failures must be represented as structured events or
  model-readable tool observations;
- full async migration requires a separate dependency-approved task.

## Implemented Guardrails

- `continue_session_tool_after_permission_with_outcome` records permission
  events under lock, executes the tool outside the session-map lock, then
  records completion after reacquiring the lock.
- `execute_session_tool` now applies the same lock discipline for read-only
  preview and FastAuto execution paths.
- `facade_executes_tool_without_blocking_session_snapshots` verifies a slow
  shell-backed tool does not block `get_session_snapshot`.
- Tauri approval submission returns a structured resume outcome instead of only
  `{ ok, session_id }`.

## Deferred Async Migration

A Tokio migration can be reconsidered only if one of these becomes true:

- provider streaming needs cancellation that cannot be implemented safely with
  the current process boundary;
- GUI needs many concurrent live sessions sharing one runtime process;
- sidecar/provider timeouts prove insufficient in real traces;
- tests show lock-free sync execution is still unable to keep snapshots/events
  responsive.

If approved, migrate one narrow path first:

1. provider transport timeout/cancellation;
2. shell command process execution;
3. approval wait/resume.

Do not add `tokio`, `tokio-util`, `futures`, or `parking_lot` as part of
unrelated runtime fixes.

## Rollback

Revert only the async migration slice if it causes test churn or event-ordering
regression. Keep provider-id preservation, permission-resume outcomes, and GUI
approval error handling; those are independent product-kernel fixes.
