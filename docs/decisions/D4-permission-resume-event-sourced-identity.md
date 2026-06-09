# D4 · Permission Resume Uses Event-Sourced Tool Identity

> Status: decided (2026-06-04)
> Trigger: shell.command approval resumed the command but stopped the agent loop.
> Scope: RuntimeFacade native-loop permission recovery.

---

## Decision

Permission resume must recover the executable tool identity from the merged
session event log, not from string-derived `permission_id` patterns.

The facade now treats the event chain as the source of truth:

1. Find the matching `permission.requested` event by `permission_id`.
2. Walk backward to the nearest matching `tool.call_requested`.
3. Use that merged `tool_call_id` and `provider_tool_call_id` for the approved
   resumed tool result.
4. Fall back to the pending tool's raw id only when the event chain is missing.

---

## Why

Native-loop event merge appends `_loop_<sequence>` to rewritable ids so repeated
turns do not collide. `permission_id` intentionally stays stable because it is
the user-facing approval handle.

The old resume path mixed those two worlds:

- native loop returned raw `tool_call_id`, for example `native_loop_v2_tool_50`;
- facade merged events as `native_loop_v2_tool_50_loop_1204`;
- approval resume then tried to infer provider identity from the raw id;
- provider id lookup failed;
- the resumed shell result was recorded without the model provider tool id;
- the GUI showed the command as done, but the model loop was not continued
  correctly.

This produced the visible failure mode where approving `shell.command` made the
turn appear completed or failed instead of feeding the command result back into
the next model call.

---

## Removed Design

The old design guessed ids from permission names such as:

- `native_loop_command_perm_N -> native_loop_tool_N`
- `native_loop_patch_perm_N -> native_loop_tool_N`

That rule is now demoted to fallback only. It cannot be the primary path because
it does not know the facade merge suffix and cannot recover provider tool-call
ids after event replay.

---

## Safety Boundary

This decision does not weaken command safety.

Dangerous commands are still blocked by the command classifier. Approval resume
only runs commands that already reached the permission gate and were allowed by
the user.

---

## Regression Tests

The runtime now has a focused test:

- `native_pending_permission_identity_uses_merged_tool_request_id`

It constructs the exact failure shape:

- pending native tool has raw id `native_loop_v2_tool_50`;
- merged event log has `native_loop_v2_tool_50_loop_1204`;
- permission id remains `native_loop_v2_command_perm_50`;
- provider id is `toolu_v2_5_0`.

The test requires the facade pending decision to use the merged tool id and the
provider id.

Existing approval execution coverage remains:

- `facade_permission_decision_executes_pending_native_tool_with_outcome`

It verifies that approved shell execution records a successful result and marks
`model_continuation_required=true`.

---

## Rollback

Rollback is local and low-risk:

1. Revert `infer_pending_native_tool_identity_from_session`.
2. Restore direct lookup through `infer_provider_tool_call_id_from_session`.
3. Remove or update `native_pending_permission_identity_uses_merged_tool_request_id`.

The expected rollback signal is that GUI shell approval may again complete the
command without continuing the model loop when ids are merge-suffixed.

