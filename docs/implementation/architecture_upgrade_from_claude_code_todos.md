# Architecture Upgrade From Claude Code: Executable TODOs

This TODO plan is derived from:

- `docs/engineering/architecture_upgrade_from_claude_code.md`
- `.claude/worktrees/gracious-dubinsky-f08f39/docs/engineering/architecture_upgrade_from_claude_code.md`
- `docs/agent_architecture_planning/39_deepseek_native_agent_kernel_architecture_and_plan.md`
- current main workspace runtime/GUI state

It replaces the stale parts of the source architecture document with tasks that
can be implemented in the current repo.

## 0. Execution Contract

Goal:

```text
Move the current runtime toward Claude-Code-grade agent-kernel reliability while
preserving DeepSeek/Qwen-native architecture.
```

Allowed scope:

- `crates/kernel`
- `crates/runtime`
- `crates/cli`
- root `desktop/`
- `scripts`
- `docs`
- focused tests and fixtures

Denied without explicit approval:

- dependency additions such as `tokio`, `parking_lot`, or `futures`;
- root `AGENTS.md`;
- destructive git operations;
- network/package-install actions;
- changing DeepSeek/Qwen native strategy without eval fixtures.

Completion gate:

- focused tests for every slice;
- `cargo test -p researchcode-runtime --lib`;
- `cargo check --manifest-path desktop/src-tauri/Cargo.toml` when Tauri changes;
- `npm run build` in `desktop/` when GUI changes;
- `python3 scripts/check_all.py` before claiming full completion.

## 1. Audit Corrections To Track

- [x] Do not target `apps/desktop` for product GUI work; target root `desktop/`.
- [x] Do not claim the Tauri bridge is missing; update tasks around existing
      `desktop/src-tauri/src/main.rs`.
- [x] Do not claim shell timeout is absent; audit remaining blocking boundaries
      instead.
- [x] Do not treat full async migration as Phase 0 unless dependency approval is
      granted.
- [x] Preserve doc39 native DeepSeek/Qwen primitives; do not genericize them into
      compatible-provider config.

Acceptance:

- [x] `docs/engineering/architecture_upgrade_from_claude_code.md` stays aligned
      with current repo paths.
- [x] This TODO file does not contain stale `apps/desktop/src-tauri` work as a
      product target.

## 2. P0: True Permission Resume Path

Problem:

Current status: implemented. The runtime now stores a facade-level pending
native decision, executes the exact pending tool after approval, records the
tool result, and returns a structured resume outcome before the GUI starts the
next model continuation.

Tasks:

- [x] Define a `RuntimePendingNativeDecision` record that stores:
  - session id;
  - permission id;
  - provider tool-use id;
  - internal tool call id;
  - tool id;
  - arguments;
  - blocked event JSONL;
  - resume strategy;
  - created timestamp.
- [x] Store the pending decision record in `RuntimeSessionRecord`, not only
      `PendingNativeToolExecution`.
- [x] Add `RuntimeFacade::resume_native_loop_after_permission_decision`.
- [x] Make `submit_permission_decision` choose between:
  - legacy session permission decision;
  - pending native decision resume;
  - ordinary facade tool continuation.
- [x] Emit structured events for:
  - `runtime.permission_resume.started`;
  - `runtime.permission_resume.tool_executed`;
  - `runtime.permission_resume.model_continued`;
  - `runtime.permission_resume.completed`;
  - `runtime.permission_resume.failed` / structured backend error event.
- [x] Make Tauri `runtime_submit_permission_decision` return the resume outcome,
      not just `{ ok, session_id }`.
- [x] Make GUI display approval failure from backend error codes without losing
      the pending card.

Tests:

- [x] RuntimeFacade test: blocked shell approval resumes and emits tool result.
- [x] RuntimeFacade test: wrong permission id fails with structured error and
      keeps pending approval.
- [x] Tauri command test or host-level smoke: approval returns non-empty resume
      outcome.
- [x] GUI unit/smoke: click approval shows in-flight state and clears only on
      success.

Acceptance:

- [x] Clicking approval produces either a tool result or a visible structured
      error.
- [x] The pending approval card cannot silently no-op.
- [x] No approval path requires repeating the original user prompt as the only
      recovery mechanism.

## 3. P0: Provider Tool-Use ID Preservation

Problem:

`ParsedToolCall` and `MediatedToolCall` do not preserve the provider's original
tool-use id end to end. Claude Code relies on exact tool-use/tool-result pairing.
DeepSeek/Qwen need the same invariant, especially under streaming and resume.

Tasks:

- [x] Add `provider_tool_call_id: Option<String>` to `ParsedToolCall`.
- [x] Add `provider_tool_call_id: Option<String>` to `MediatedToolCall`.
- [x] Preserve provider ids from:
  - OpenAI-compatible `tool_calls[].id`;
  - Anthropic-compatible `tool_use.id`;
  - streaming `ToolCallStarted.id`;
  - DSML/content fallback when an id exists.
- [x] Keep internal `tool_call_id` for ledger identity, but use provider id for
      provider tool-result replay when available.
- [x] Emit both ids in tool lifecycle events:
  - `tool.call_requested`;
  - `tool.call_assembled`;
  - `tool.schema.checked`;
  - `tool.call_completed`;
  - `tool.result_recorded`.
- [x] Update conversation history projection to preserve provider ids.
- [x] Update DeepSeek/Qwen tool-result request builders to replay with original
      provider id.

Tests:

- [x] Parser test for OpenAI-compatible id preservation.
- [x] Parser/DSML test for native fallback id preservation.
- [x] Streaming assembler test for id preservation across partial JSON chunks.
- [x] Native loop test: three sequential tool calls replay with matching ids.
- [x] Resume test: pending tool result uses the original provider id.

Acceptance:

- [x] No model-visible tool result uses a fabricated id when the provider gave an
      id.
- [x] Internal ids remain stable for event replay and GUI cards.

## 4. P0: Lock And Blocking Boundary Audit

Problem:

The repo now has shell timeout, but blocking risk remains around session locks,
tool execution, sidecar process calls, provider transport, and Tauri command
continuation.

Tasks:

- [ ] Inventory every blocking call in:
  - `crates/runtime/src/runtime_facade.rs`;
  - `crates/runtime/src/tool_execution.rs`;
  - `crates/runtime/src/command.rs`;
  - `crates/runtime/src/live_http_transport.rs`;
  - `crates/runtime/src/sidecar_http_transport.rs`;
  - `crates/runtime/src/research_worker.rs`;
  - `desktop/src-tauri/src/main.rs`.
- [ ] Mark whether each blocking call happens under:
  - global session map lock;
  - single session lock;
  - no runtime lock.
- [ ] Refactor `execute_session_tool` so tool execution does not hold the global
      sessions mutex across the executor call.
- [ ] Refactor any remaining facade method that runs external process/provider
      work while holding the sessions mutex.
- [ ] Add timeout config defaults for:
  - shell command;
  - git command;
  - research worker;
  - provider sidecar spawn/status;
  - provider HTTP call;
  - Tauri approval continuation.
- [ ] Emit `runtime.blocking_boundary.timeout` with call site and duration when a
      timeout fires.

Tests:

- [ ] Shell timeout test remains passing.
- [ ] Add test: blocked tool execution does not block `get_session_snapshot`.
- [ ] Add test: one hung session does not block a second session's snapshot or
      event stream.
- [ ] Add test: sidecar/provider timeout becomes recoverable event, not process
      hang.

Acceptance:

- [ ] No known long-running provider/tool/process call is executed while holding
      the global sessions mutex.
- [ ] Every external process/provider boundary has timeout and structured error.

## 5. P0: GUI Approval Reliability

Problem:

The GUI must never show an approval button that can silently fail.

Tasks:

- [x] Keep `permissionDecisionInFlight` state for individual approval cards.
- [x] Preserve pending card on backend failure.
- [x] Add visible error message near the approval card, not only transcript text.
- [x] Disable duplicate clicks while the same permission is in flight.
- [x] Add a retry button after failure.
- [x] Add `runtime_submit_permission_decision` error code display mapping.
- [x] When Tauri transport is unavailable, show bridge-not-ready instead of
      silently returning.
- [x] Include build mark and transport type in debug event or GUI diagnostics.

Tests:

- [x] `npm run build`.
- [x] Component-level test or static smoke for missing bootstrap.
- [x] Manual Tauri smoke: approval success clears card.
- [x] Manual Tauri smoke: forced backend error leaves card and shows error.

Acceptance:

- [x] Approval click always gives immediate visual feedback.
- [x] A failed approval is visible and retryable.

## 6. P1: Streaming Tool Input Lifecycle

Problem:

Streaming tool-call accumulation exists, but GUI/event consumers do not get a
complete lifecycle.

Tasks:

- [ ] Add canonical events:
  - `tool.input_started`;
  - `tool.input_delta`;
  - `tool.input_finalized`;
  - `tool.input_repaired`;
  - `tool.input_rejected`.
- [ ] Emit streaming lifecycle events from the native stream handler.
- [ ] Do not expose sensitive arguments in GUI-visible payloads for side-effect
      tools.
- [ ] Only dispatch safe read-only streaming tools before full assistant message
      completion.
- [ ] Gate write/shell/control tools through normal permission flow even when
      their input finalizes early.

Tests:

- [ ] Streaming partial JSON five-chunk fixture.
- [ ] Parallel streaming tool index fixture.
- [ ] Sensitive side-effect argument redaction fixture.
- [ ] GUI event replay fixture for streaming input cards.

Acceptance:

- [ ] GUI can render tool input assembly without guessing from final text.
- [ ] Streaming lifecycle does not duplicate execution.

## 7. P1: Compaction And Reasoning Replay Under Long Sessions

Problem:

Compaction and budget telemetry exist, but long-session behavior must prove that
DeepSeek reasoning replay and tool evidence survive compaction.

Tasks:

- [ ] Add a long-session fixture that crosses the 192K compaction threshold via
      synthetic context.
- [ ] Preserve latest required DeepSeek raw reasoning for adjacent tool-result
      replay.
- [ ] Store sanitized reasoning preview separately from raw provider replay.
- [ ] Emit:
  - `context.compaction.started`;
  - `context.compaction.completed`;
  - `context.compaction.blocked`;
  - `reasoning.replay.required`;
  - `reasoning.replay.injected`;
  - `reasoning.replay.missing`.
- [ ] Add GUI/TUI rendering for compaction events as lifecycle cards.

Tests:

- [ ] DeepSeek thinking + tool + compaction + next request fixture.
- [ ] Missing reasoning replay blocks before HTTP call.
- [ ] GUI replay shows compaction without raw reasoning leak.

Acceptance:

- [ ] Required reasoning replay is never compacted away before provider replay.
- [ ] GUI never displays raw reasoning.

## 8. P1: Error Taxonomy And Retry Policy

Problem:

Many runtime errors are still plain strings. Claude-Code-grade loops need
retryable/fatal/user-cancelled distinctions.

Tasks:

- [ ] Introduce `RuntimeErrorKind` or equivalent non-breaking error wrapper.
- [ ] Classify:
  - provider HTTP timeout;
  - provider 429/5xx;
  - provider 400 tool-result protocol mismatch;
  - permission denied;
  - tool schema validation failure;
  - tool execution timeout;
  - user cancellation.
- [ ] Convert native loop recovery branches to use classification.
- [ ] Emit `runtime.error.classified` with retryability and suggested next
      action.
- [ ] Add fallback policy for retryable provider errors.

Tests:

- [ ] HTTP 429 fixture.
- [ ] HTTP 5xx fixture.
- [ ] HTTP 400 tool-result mismatch fixture.
- [ ] Tool timeout fixture.
- [ ] User cancel fixture.

Acceptance:

- [ ] Recoverable errors stay model-readable and event-visible.
- [ ] Fatal errors do not trigger repeated tool/model loops.

## 9. P1: Event Store And Approval Queue Incrementality

Problem:

Approval queue extraction and GUI replay should scale with long sessions.

Tasks:

- [ ] Audit current event persistence path and document exact source of truth.
- [ ] Add per-session approval queue cursor or cached projection.
- [ ] Ensure pending permission state can be rebuilt from event log after
      process restart.
- [ ] Add atomic event append path for product sessions if not already used.
- [ ] Add rotation policy for large session logs.

Tests:

- [ ] Rebuild pending approval after replay.
- [ ] Replay 10K event log without full GUI freeze.
- [ ] Rotation keeps hash/sequence validation intact.

Acceptance:

- [ ] Event log remains single truth source.
- [ ] Approval queue is replayable and efficient.

## 10. P2: Async Runtime ADR

Problem:

Full async migration may be the right long-term architecture, but it requires
dependency approval and careful test migration.

Tasks:

- [ ] Write ADR: keep sync runtime with bounded blocking vs migrate to tokio.
- [ ] Estimate changed files and test migration cost.
- [ ] Define migration compatibility layer so existing sync tests keep running.
- [ ] If approved, add dependencies in a dedicated commit:
  - `tokio`;
  - `tokio-util`;
  - `futures`;
  - optionally `parking_lot`.
- [ ] Convert one narrow path first:
  - provider transport timeout; or
  - shell command process execution; or
  - approval wait/resume.

Acceptance:

- [ ] No async dependency is added accidentally inside unrelated runtime fixes.
- [ ] ADR records rollback condition and eval impact.

## 11. P2: Hooks

Tasks:

- [ ] Add hook event enum:
  - `UserPromptSubmit`;
  - `PreToolUse`;
  - `PostToolUse`;
  - `PostToolUseFailure`;
  - `Stop`;
  - `PreCompact`;
  - `PostCompact`.
- [ ] Add hook timeout and failure policy.
- [ ] Add hook result types:
  - allow;
  - deny;
  - modify;
  - warn.
- [ ] Emit hook lifecycle events.
- [ ] Ensure hooks cannot bypass PermissionGate.

Tests:

- [ ] PreToolUse deny blocks dispatch.
- [ ] Hook timeout allows with warning.
- [ ] Hook modify updates args before schema validation.

Acceptance:

- [ ] Hooks are observability/control points, not permission bypasses.

## 12. P2: Subagent Isolation

Tasks:

- [ ] Give each subagent an isolated session/event log.
- [ ] Ensure subagent tools do not share parent mutable state except through
      explicit result events.
- [ ] Add per-subagent budget and timeout.
- [ ] Add parent event for subagent result merge.
- [ ] Preserve AGENTS.md banned multi-agent core path rules.

Tests:

- [ ] Explorer read-only isolation.
- [ ] Worker write-scope enforcement.
- [ ] Parent can continue if subagent times out.

Acceptance:

- [ ] Subagent failure cannot freeze parent runtime.

## 13. Final Verification Matrix

Before claiming this upgrade complete:

- [ ] `cargo fmt --all`
- [ ] `cargo test -p researchcode-runtime --lib`
- [ ] `cargo test --workspace`
- [ ] `cargo check --manifest-path desktop/src-tauri/Cargo.toml`
- [ ] `npm run build` in `desktop/`
- [ ] `python3 scripts/claudecode_gap_check.py`
- [ ] `python3 scripts/check_all.py`

Manual checks:

- [ ] Start GUI with `npm run tauri:dev`.
- [ ] Ask model to run a safe shell command.
- [ ] Approval card appears.
- [ ] Clicking allow gives immediate in-flight state.
- [ ] Shell tool completes or visible structured error appears.
- [ ] Multi-tool long run continues to final answer.

## 14. Progress Ledger

Use this section during implementation. Do not mark a phase complete unless all
acceptance items and tests pass.

| Phase | Status | Evidence |
|---|---|---|
| P0 true permission resume | implemented | `RuntimePendingNativeDecision`, provider-id-aware pending tools, `submit_permission_decision_with_outcome`, Tauri structured outcome, GUI error preservation, `runtime.permission_resume.*` events; verified by `facade_approval_decision_executes_pending_native_shell_tool` and `python3 scripts/check_all.py`. |
| P0 provider tool-use id preservation | implemented | `ParsedToolCall`/`MediatedToolCall` carry provider ids; event payloads and conversation history replay preserve them; parser/native-loop tests and `check_all.py` pass. |
| P0 lock/blocking boundary audit | implemented for facade tool/approval execution | Approval continuation and `execute_session_tool` no longer execute tools while holding the global session map lock; `facade_executes_tool_without_blocking_session_snapshots` proves snapshot access during a slow shell tool. Remaining async-runtime migration is deferred to ADR. |
| P0 GUI approval reliability | implemented | Per-permission in-flight state, backend error preservation, visible card-level error, retry button, batch failure mapping, Tauri bridge diagnostics, `npm run build` pass. |
| P1 streaming tool lifecycle | implemented baseline | Native stream handler emits `tool.input_started`, `tool.input_delta`, `tool.input_finalized`, and rejected/repaired paths; lifecycle payloads keep raw args out of GUI-visible stream events; native streaming fixture passes. |
| P1 compaction/reasoning replay long-session proof | partial existing coverage | Native controller already emits `context.compaction.*`; long DeepSeek tool-run/context guard fixture passes. Full GUI compaction card remains a follow-up. |
| P1 error taxonomy/retry policy | partial existing coverage | Existing tool/model-readable errors are structured and HTTP 400 recovery is covered; a unified `RuntimeErrorKind` wrapper is still future work. |
| P1 event store/approval queue incrementality | partial existing coverage | Event logs and approval queues replay from JSONL, including blocked/resumed native-loop fixtures; 10K GUI replay and rotation policy remain future work. |
| P2 async runtime ADR | implemented | `docs/engineering/adr_sync_runtime_bounded_blocking_vs_tokio.md` records sync bounded-blocking decision, migration trigger, rollback condition, and dependency guard. |
| P2 hooks | deferred | Not implemented in this slice; must remain behind PermissionGate. |
| P2 subagent isolation | partial existing coverage | Read-only/worker isolation tests and policy exist; fully isolated child event logs remain future work. |
