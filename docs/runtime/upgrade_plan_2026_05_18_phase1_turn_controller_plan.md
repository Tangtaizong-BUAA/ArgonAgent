# Upgrade Plan 2026-05-18 Phase 1 TurnController Repair Plan

Source contract:

- `/Users/gongyuxuan/Documents/deep-code/upgrade_plan_2026_05_18.md`
- `/Users/gongyuxuan/Documents/deep-code/docs/runtime/upgrade_plan_2026_05_18_p0_execution_plan.md`
- `/Users/gongyuxuan/Documents/deep-code/docs/runtime/upgrade_plan_2026_05_18_p1_p2_execution_plan.md`

Phase 1 target:

- Turn the current native loop body into a real `agent_kernel::turn_controller`
  implementation instead of a facade.
- Keep `run_native_agent_loop_v2_deepseek_inner` as the compatibility wrapper
  that wires transport, request, session creation, and final loop result.
- Preserve DeepSeek/Qwen native behavior, event replay shape, permission
  semantics, and P0/P1/P2 loop-stability repairs.

Current baseline observed on 2026-05-18:

- `crates/runtime/src/native_agent_loop.rs`: 13,943 lines.
- `crates/runtime/src/agent_kernel/turn_controller.rs`: 12 lines, facade-only.
- `crates/runtime/src/native_turn_controller.rs`: 585 lines, ledger/context
  guard helper rather than an iteration controller.
- `run_native_agent_loop_v2_deepseek_inner` still owns the turn loop, request
  preparation, streaming tool execution, parsed tool execution, convergence
  handling, finalization, and fallback behavior.

## TaskContract

Goal:

- Complete Phase 1 TurnController extraction at engineering quality, not as a
  rename or facade reshuffle.

Allowed paths:

- `crates/runtime/src/native_agent_loop.rs`
- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/native_turn_controller.rs`
- `crates/runtime/src/agent_kernel/mod.rs`
- focused runtime tests under `crates/runtime/src/**`
- runtime docs under `docs/runtime/**`

Conditionally allowed paths:

- `crates/runtime/src/agent_kernel/turn_state.rs` only for small interface
  changes required by the controller.
- `crates/runtime/src/context_budget.rs` only if controller boundaries expose a
  budget drift or missing helper.

Denied paths and work:

- root `AGENTS.md`
- provider-layer redesign
- model-router redesign
- full TCML migration
- permission-manager redesign beyond preserving existing request-shaped entry
- GUI changes
- database/event schema migrations
- dependency installs, lockfile churn, network calls, destructive git commands

Allowed tools:

- read/search/edit/patch;
- non-destructive local cargo checks and focused tests;
- `git status` / `git diff` for inspection only unless a checkpoint is
  explicitly requested.

Required verification:

- focused tests after each coherent extraction slice;
- `cargo fmt --check`;
- `cargo test -p researchcode-runtime --lib -- --test-threads=1`;
- `cargo test -p researchcode-kernel --lib -- --test-threads=1`.

Known verification caveat:

- runtime local API server tests may need non-sandbox execution because they
  bind `127.0.0.1`. If sandbox blocks them, rerun the same cargo command with
  approved escalation rather than weakening the test set.

Stop conditions:

- extraction requires changing public event schema or GUI-consumed event order;
- permission/patch safety semantics would need redesign;
- DeepSeek/Qwen native behavior would be collapsed into compatible-provider
  generic behavior;
- tests expose a blocker whose fix belongs to Phase 2+ scope;
- dependency install or network access becomes required.

## Non-Negotiable Invariants

- Exactly-once tool execution must be preserved for streamed and parsed tool
  calls.
- Provider tool-call ids must remain paired with their model-readable tool
  results.
- Duplicate observations must stay suppressed from `last_tool_batch`; novelty
  and plateau checks must continue to use ObservationCache distinct-key growth.
- `EvidenceLedger` and legacy `last_tool_batch` keep their current split until
  the controller owns the iteration boundary and can safely retire the legacy
  replay vector.
- `PermissionDecision::Deny` remains model-readable and must not become a hard
  loop error again.
- Fast auto write and CodeEdit paths must keep dangerous-path checks.
- Context compaction must rebuild model requests explicitly and keep replayable
  runtime events.
- A completed turn must produce visible assistant text or a visible fallback;
  the controller must not reintroduce "completed but no displayable text".

## Extraction Strategy

Phase 1 is intentionally incremental. The first pass should introduce real
controller types and move narrow decisions; later passes move side-effect-heavy
branches. Each slice must compile and keep tests green before continuing.

### Slice 0: Baseline Guardrail

Purpose:

- Freeze observed behavior before moving code.

Work:

- Record line counts and key loop boundaries in this plan and implementation
  notes.
- Identify current tests covering duplicate reads, streamed/parsed mismatch,
  permission denial, DSML fallback, context budget, and native loop completion.
- Add missing focused regression tests only if an extraction would otherwise be
  untestable.

Verification:

- `cargo fmt --check`
- focused existing tests for P0/P1/P2 hot paths.

Rollback condition:

- No production code should change in this slice.

Done when:

- There is a known baseline for line count, facade status, and hot-path tests.

### Slice 1: Real Controller Contract

Purpose:

- Replace facade-only `agent_kernel::turn_controller` with real Phase 1 types
  while preserving the existing `NativeTurnController` ledger helper.

Work:

- Define `TurnController`, `TurnContext`, `IterationOutcome`, and
  `FinalizationReason` in `agent_kernel/turn_controller.rs`.
- Keep compatibility exports for `NativeTurnController` until callers migrate.
- Make `TurnContext` borrow rather than own heavyweight runtime objects:
  session, artifact store, endpoint, context budget, tool manifest, turn state,
  evidence ledger, legacy batch, counters, event sink cursor, and interrupt.
- Start with methods that are pure or low side effect:
  iteration id creation, interrupt check, budget check, and loop counter sync.

Expected files:

- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/agent_kernel/mod.rs`
- narrow call-site updates in `crates/runtime/src/native_agent_loop.rs`

Verification:

- compile-focused runtime tests;
- focused tests for max tool-call budget fallback and interrupt behavior if
  already present.

Rollback condition:

- Borrowing shape forces broad lifetime churn outside runtime hot path.

Done when:

- `agent_kernel::turn_controller` is no longer facade-only;
- the native loop calls a controller method for at least the iteration preflight
  gate.

### Slice 2: Budget and Finalization Gate Extraction

Purpose:

- Move the safest loop gates out of the monolith first.

Work:

- Extract max-iteration/max-tool-call handling into controller methods that
  return `IterationOutcome::Finalize` or `IterationOutcome::Block`.
- Keep actual finalizer model calls in wrapper until finalizer dependencies are
  represented cleanly in `TurnContext`.
- Extract "no last tool batch means failed budget" decision without changing
  current failure events.
- Preserve `agent.loop_budget_reached` and `agent.loop_incomplete` payloads.

Expected files:

- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/native_agent_loop.rs`

Verification:

- focused native loop tests for max tool calls / max iterations;
- `cargo test -p researchcode-runtime --lib turn_state -- --test-threads=1`.

Rollback condition:

- event payload ordering or status changes for budget failure/completion.

Done when:

- main loop no longer contains inline max-tool-call/max-iteration decision
  logic except for finalizer invocation.

### Slice 3: Request Preparation Plan

Purpose:

- Separate model request planning from loop mechanics.

Work:

- Add a controller-owned `IterationRequestPlan` enum:
  `Initial`, `Continuation`, `CompactedInitial`, `CompactedContinuation`,
  `EvidenceRetry`.
- Move strategy selection for plain evidence continuation vs provider
  `tool_result` continuation into the controller.
- Keep actual HTTP send in `native_agent_loop.rs` during this slice.
- Preserve DeepSeek reasoning replay and cache-zone telemetry calls.
- Preserve context-guard behavior and compaction retry events.

Expected files:

- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/native_agent_loop.rs`

Verification:

- focused tests around continuation request construction;
- existing context budget tests;
- any DeepSeek reasoning replay tests already present.

Rollback condition:

- request preview, cache breakpoint, or reasoning replay changes unexpectedly.

Done when:

- initial vs continuation request construction is represented by a controller
  plan object, and wrapper sends the prepared request selected by that plan.

### Slice 4: Stream Result Normalization

Purpose:

- Give the controller one normalized model-result input per iteration.

Work:

- Introduce `IterationModelResponse` containing:
  call id, stream id, transcript id, HTTP status, recorded response, streamed
  tool batch, pending permission/tool state, and streamed suppression count.
- Move post-send decisions into controller:
  pending tool block, HTTP failure recovery choice, DSML leak event decision,
  streamed/parsed mismatch decision, empty-visible finalization decision.
- Keep `send_with_live_visible_stream_events` and low-level stream handler in
  `native_agent_loop.rs` until Phase 4 StreamProcessor work.

Expected files:

- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/native_agent_loop.rs`

Verification:

- streamed/parsed mismatch regression;
- DSML fallback tests;
- empty visible response / visible fallback tests if present.

Rollback condition:

- streamed tool calls execute twice, or parsed tool calls lose model-readable
  error pairing.

Done when:

- the main loop hands a normalized response into the controller and receives an
  explicit continue/finalize/block decision.

### Slice 5: Tool Dispatch Extraction

Purpose:

- Move the largest source of loop complexity into the controller.

Work:

- Extract parsed tool-call dispatch in dependency order:
  manifest recovery;
  model-readable contract errors;
  plan.enter / plan.exit;
  ask_user;
  agent.final_answer;
  permissioned write;
  permissioned shell;
  duplicate read-only suppression;
  read-only execution;
  non-progress recovery.
- Keep execution helper functions in `native_agent_loop.rs` initially if moving
  them would create unrelated churn; controller can call helpers through a
  narrow dispatch trait or function table.
- Ensure `TurnContext` owns the mutation points:
  `tool_call_count`, `EvidenceLedger`, `last_tool_batch`,
  `ObservationCache`, `NativeTurnController`, and event emission cursor.
- Preserve concurrent read-only execution behavior and per-iteration cap.

Expected files:

- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/native_agent_loop.rs`
- possible focused tests in runtime modules.

Verification:

- duplicate read plateau tests;
- permission denial model-readable tests;
- dangerous path tests;
- streamed/parsed mismatch tests;
- read-only concurrent batch tests if present.

Rollback condition:

- tool count, provider id, artifact recording, or permission pending state
  differs from baseline.

Done when:

- parsed tool dispatch is no longer an inline branch forest inside
  `run_native_agent_loop_v2_deepseek_inner`.

### Slice 6: Convergence and Recovery Ownership

Purpose:

- Move loop-progress decisions into the controller where evidence state lives.

Work:

- Move repeated batch detection, duplicate cached observation handling,
  `ToolProgressState`, `ConvergenceEnforcer`, escalation-to-CodeEdit, and
  plateau finalization decisions behind controller methods.
- Preserve ObservationCache distinct-key growth as the only "new evidence"
  source for plateau decisions.
- Keep finalizer model call helper in wrapper unless `TurnContext` has already
  absorbed all dependencies safely.

Expected files:

- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `crates/runtime/src/native_agent_loop.rs`
- possibly `crates/runtime/src/agent_kernel/turn_state.rs` for interface
  cleanup only.

Verification:

- duplicate read plateau regression tests;
- full runtime lib tests;
- event-log replay inspection with a previous loop-heavy fixture if available.

Rollback condition:

- repeated tool batches stop finalizing, or duplicate suppression becomes model
  visible again.

Done when:

- convergence and progress decisions are controller-owned, not scattered after
  tool dispatch.

### Slice 7: Thin Wrapper Cleanup

Purpose:

- Make `native_agent_loop.rs` visibly stop being the product kernel.

Work:

- Collapse `run_native_agent_loop_v2_deepseek_inner` into:
  session setup;
  manifest setup;
  controller construction;
  loop calling `controller.run_iteration`;
  final conversion to `NativeAgentLoopResult`.
- Move helper functions only when their ownership is clear and tests cover the
  move.
- Update facade comments to reflect real ownership.
- Update this plan with completion evidence.

Expected files:

- `crates/runtime/src/native_agent_loop.rs`
- `crates/runtime/src/agent_kernel/turn_controller.rs`
- `docs/runtime/upgrade_plan_2026_05_18_phase1_turn_controller_plan.md`

Verification:

- `cargo fmt --check`
- `cargo test -p researchcode-runtime --lib -- --test-threads=1`
- `cargo test -p researchcode-kernel --lib -- --test-threads=1`
- final `wc -l` for `native_agent_loop.rs`,
  `agent_kernel/turn_controller.rs`, and `native_turn_controller.rs`.

Rollback condition:

- wrapper cleanup creates churn without measurable line reduction or ownership
  clarity.

Done when:

- `agent_kernel/turn_controller.rs` owns iteration orchestration;
- `run_native_agent_loop_v2_deepseek_inner` is a thin compatibility wrapper;
- `native_agent_loop.rs` line count has materially fallen;
- original Phase 1 acceptance is either met or remaining line-count gap is
  explicitly documented as helper relocation that is safe for a follow-up.

## Acceptance Criteria

Hard acceptance:

- `agent_kernel/turn_controller.rs` contains a real controller implementation,
  not only re-exports.
- `run_native_agent_loop_v2_deepseek_inner` no longer owns request strategy,
  tool dispatch, and convergence decisions inline.
- Existing P0/P1/P2 repairs remain intact.
- Focused tests for the moved behavior pass.
- Broad runtime and kernel lib tests pass.

Line-count acceptance:

- Target from source plan: `native_agent_loop.rs` below 3000 lines and
  `agent_kernel/turn_controller.rs` above 600 lines.
- If below-3000 cannot be reached without moving unrelated helper libraries, the
  implementation must still produce a major measured reduction and document the
  exact remaining helpers that belong to Phase 2/3/4 rather than Phase 1.

Behavioral acceptance:

- A long task can continue through observe -> decide -> execute -> feed back ->
  finalize without reporting "completed but no displayable text".
- Duplicate observations do not inflate evidence.
- Tool failures produce model-readable feedback unless they are true runtime
  blockers.
- Permission pending states block cleanly with replayable events.
- GUI-visible events remain replayable and ordered.

## Risk Register

1. Mutable session state is broad.
   Mitigation: borrow `AgentSession` through `TurnContext`; do not clone or
   invent a parallel session model.

2. Event emission cursor can drift.
   Mitigation: keep `emitted_event_count` in `TurnContext` and emit through one
   helper.

3. Streamed and parsed tool paths can double execute.
   Mitigation: move streamed mismatch tests before tool dispatch extraction and
   preserve streamed batch precedence.

4. Permission pending can be mistaken for failure.
   Mitigation: represent pending as `IterationOutcome::Block` with pending tool
   payload, not as `Err`.

5. Context compaction request rebuilds are easy to break.
   Mitigation: isolate request-plan extraction before HTTP send extraction.

6. `last_tool_batch` vs `EvidenceLedger` split is transitional.
   Mitigation: do not retire `last_tool_batch` until finalizer and provider
   replay paths are controller-owned.

7. Line-count target may tempt unsafe bulk moves.
   Mitigation: move orchestration first; helper relocation is allowed only with
   tests and clear ownership.

## Implementation Cadence

- Keep one active checklist while implementing.
- After each slice:
  - run focused tests;
  - repair failures immediately;
  - record completion evidence in this document;
  - continue to the next unblocked slice without asking for another "continue".
- Run broad checks only after all unblocked slices are complete.
- Stop only on the TaskContract stop conditions above.

## Final Report Format

The final implementation report must include:

- changed files;
- slice-by-slice completion status;
- line-count before/after;
- tests/checks run with results;
- preserved invariants;
- risks and any remaining gaps;
- next recommended task.

## Implementation Evidence 2026-05-18

Implemented in this pass:

- Slice 0 baseline recorded.
  - before: `native_agent_loop.rs` 13,943 lines;
  - before: `agent_kernel/turn_controller.rs` 12 lines, facade-only.
- Slice 1 partially completed.
  - `agent_kernel::turn_controller` is no longer facade-only;
  - added real controller surface:
    `TurnController`, `IterationOutcome`, `FinalizationReason`,
    `IterationPreflight`, `NativeLoopIterationIds`,
    `NativeLoopTurnController`, `ToolProgressReport`,
    `LoopConvergenceAction`, and `ContinuationStrategy`;
  - kept `NativeTurnController` compatibility exports for the existing
    ledger/context guard helper.
- Slice 2 partially completed.
  - main loop iteration preflight now calls
    `NativeLoopTurnController::begin_iteration`;
  - interrupt, iteration counter sync, max-tool-call failure, and
    max-tool-call finalization event creation moved out of
    `run_native_agent_loop_v2_deepseek_inner`.
- Slice 3 partially completed.
  - continuation strategy event and DeepSeek reasoning replay event creation
    moved into `NativeLoopTurnController`;
  - actual request construction and HTTP send remain in the wrapper.
- Slice 6 partially completed.
  - repeated batch recovery count and exhausted repeated-batch finalization
    event creation moved into `NativeLoopTurnController`;
  - duplicate suppression summary event creation moved into controller;
  - progress plateau decision is now made through controller-owned
    ObservationCache distinct-key growth reporting;
  - convergence verdict handling, escalation accounting, and finalization event
    creation moved into controller;
  - non-progress recovery count and repeated non-progress event creation moved
    into controller.

Current line counts after this pass:

- `crates/runtime/src/native_agent_loop.rs`: 13,749 lines.
- `crates/runtime/src/agent_kernel/turn_controller.rs`: 651 lines.

Acceptance status:

- Met: `agent_kernel/turn_controller.rs` is above 600 lines and contains real
  controller behavior.
- Met: P0/P1/P2 loop-stability repairs stayed intact under focused and broad
  tests.
- Not yet met: `native_agent_loop.rs` is not below 3000 lines. The remaining
  work is the large Slice 4/Slice 5 extraction of model-response normalization
  and parsed tool dispatch. Those branches are still side-effect-heavy and must
  be moved behind a dispatch interface rather than bulk-cut blindly.

Verification run:

- `cargo fmt --check`: passed.
- `cargo test -p researchcode-runtime --lib turn_controller -- --test-threads=1`:
  passed, 11 tests.
- `cargo test -p researchcode-runtime --lib turn_state -- --test-threads=1`:
  passed, 3 tests.
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_streamed_parsed_mismatch -- --test-threads=1`:
  passed, 1 test.
- `cargo test -p researchcode-runtime --lib permission_policy -- --test-threads=1`:
  passed, 7 tests.
- `cargo test -p researchcode-runtime --lib dsml -- --test-threads=1`:
  passed, 17 tests.
- `cargo test -p researchcode-kernel --lib -- --test-threads=1`:
  passed, 27 tests.
- `cargo test -p researchcode-runtime --lib -- --test-threads=1`:
  passed, 557 tests. The first sandboxed run failed only on
  `local_api_server` tests because binding `127.0.0.1:0` was denied; the same
  command passed with approved escalation.

Next required slice:

- Slice 4/Slice 5: introduce a `ToolDispatchContext` or dispatch trait so the
  parsed tool-call branch forest can move out of `native_agent_loop.rs` without
  changing exactly-once execution, provider id pairing, pending permission
  blocking, or artifact recording.

## Implementation Evidence 2026-05-18 Continued

Implemented in the continuation pass:

- Slice 4 partially completed.
  - streamed batch ready event creation moved into `NativeLoopTurnController`;
  - pre-append streamed/parsed mismatch semantics preserved so the event still
    reports `continue_with_streamed_results_size_mismatch` after synthetic
    mismatch results are appended;
  - executable DSML fallback, DeepSeek DSML leak counting, and leak escalation
    event creation moved into `NativeLoopTurnController`;
  - empty visible response recovery count and event creation moved into
    `NativeLoopTurnController`.
- Slice 5 preparatory work completed.
  - parsed tool batch signature classification moved into
    `NativeLoopTurnController`;
  - repeated cached observation batch event creation moved into controller;
  - novel batch remembering moved behind a controller method that prevents
    accidental duplicate signature insertion.

Current line counts after continuation:

- `crates/runtime/src/native_agent_loop.rs`: 13,666 lines.
- `crates/runtime/src/agent_kernel/turn_controller.rs`: 896 lines.

Additional verification run:

- `cargo fmt --check`: passed.
- `cargo test -p researchcode-runtime --lib turn_controller -- --test-threads=1`:
  passed, 15 tests.
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_recovers_from_repeated_tool_batch -- --test-threads=1`:
  passed, 1 test.
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_streamed_parsed_mismatch -- --test-threads=1`:
  passed, 1 test.
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_empty_visible_after_tools_synthesizes_evidence_summary -- --test-threads=1`:
  passed, 1 test.
- `cargo test -p researchcode-runtime --lib dsml -- --test-threads=1`:
  passed, 18 tests.
- `cargo test -p researchcode-kernel --lib -- --test-threads=1`:
  passed, 27 tests.
- `cargo test -p researchcode-runtime --lib -- --test-threads=1`:
  passed, 561 tests with approved escalation for `local_api_server` loopback
  binding. The sandboxed run again failed only on loopback bind permission.

Remaining Phase 1 work:

- Move the side-effecting parsed tool dispatch branch behind a
  `ToolDispatchContext` or equivalent dispatch trait.
- Move finalizer invocation routing after controller outcomes once dispatch
  side effects are represented explicitly.
- The line-count target remains unmet until that dispatch extraction lands.
