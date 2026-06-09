# Current Implementation Replan After Full Audit Read

Date: 2026-06-04
Status: active execution overlay
Scope: agent loop, permission resume, TCML, provider projection, context spine, GUI lifecycle, and AgentKernel migration.

This document is the current implementation plan after reading the audit package:

- `00_scope.md`
- all `02_agent_reports/01-23`
- `03_issue_matrix.md`
- `04_architecture_review.md`
- `04_doc39_conflict_matrix.md`
- `05_test_system_review.md`
- `06_remediation_grading.md`
- all `05_red_team/*`
- `08_final_audit_package.md`
- `09_corrected_architecture_remediation_plan.md`

It does not rewrite the audit evidence. It reconciles that evidence with the
current code state and defines the next work order.

## 0. 2026-06-04 Planning Correction Checkpoint

After rereading the full audit package, doc39, and the corrected remediation
plan, the execution contract is:

1. Treat `08_final_audit_package.md`, `03_issue_matrix.md`, and
   `04_doc39_conflict_matrix.md` as evidence snapshots. They may describe
   issues that are already fixed in the current dirty worktree.
2. Treat `09_corrected_architecture_remediation_plan.md` as the architectural
   invariant document.
3. Treat this file as the live execution overlay. When it conflicts with
   `09` on ordering, this file wins only because it has been reconciled against
   current implementation status.
4. Do not begin AgentKernel ownership migration while provider projection,
   TCML sole-path behavior, loop state, context spine, GUI lifecycle, and
   deterministic harness gates still have open items.

### 0.1 Full Audit Landing Reconciliation

The audit package is now treated as fully read and landed. Planning therefore
uses three buckets:

1. **Evidence snapshot:** `03_issue_matrix.md`, `04_doc39_conflict_matrix.md`,
   `04_architecture_review.md`, `05_test_system_review.md`,
   `06_remediation_grading.md`, all `02_agent_reports/*`, all
   `05_red_team/*`, and `08_final_audit_package.md`.
2. **Architecture invariant:** `09_corrected_architecture_remediation_plan.md`
   plus doc39/doc40. These define what must not regress:
   no finalizer tool, no visible finalizer completion primitive, no tool hiding
   as workflow steering, no hard loop cap as normal control, and no compatible
   provider behavior overriding DeepSeek/Qwen native profiles.
3. **Live execution overlay:** this file. It reclassifies evidence against the
   dirty worktree and decides the next implementation order.

The most important planning correction is that quick wins are no longer
executed as a flat list. They are folded into the correctness spine below:

```text
event/permission truth
  -> TCML sole path
  -> provider projection and stream observability
  -> explicit loop state and convergence authority
  + GUI lifecycle truth
  + deterministic/live stress gates
  -> context spine
  -> AgentKernel ownership migration
```

Anything outside that chain is deferred unless it blocks one of these gates.

### 0.2 Current Phase A Status

- Implemented locally: concurrent read-only tool batches now enter TCML before
  dispatch, emit mediation telemetry before `tool.call_requested`, use
  canonical tool IDs, preserve canonical argument JSON, and convert mediation
  rejection into model-readable tool observations.
- Additional fix required by the focused gate: canonical argument serialization
  in the native loop must preserve all `ParsedToolArguments` fields, including
  `offset`, `limit`, `max_bytes`, search bounds, edit metadata, task role, and
  write scope. This prevents TCML repairs from being lost between mediation and
  provider continuation replay.
- Monitor review fix: the concurrent branch must enforce read-only eligibility
  before dispatch. `ToolRisk::WritesFiles`, `ToolRisk::ExecutesCommand`, and
  interactive/unknown tools remain unhandled by the concurrent branch and fall
  through to the serial native loop, preserving permission and pending-decision
  semantics. Non-read-only tools are also ordering barriers: the concurrent
  branch may batch only the consecutive read-only prefix from the original
  model tool-call order, and must not execute later reads ahead of an
  intervening write/shell/interactive tool.
- Focused verification: `cargo test -p researchcode-runtime tcml::contract`
  and `cargo test -p researchcode-runtime native_agent_loop_v2_concurrent_read_only`
  pass after the serialization fix. The concurrent focused suite also includes
  a write-tool regression proving `file.write` does not enter read-only
  concurrent execution, plus a `read -> write -> read` regression proving later
  reads are not executed before the pending write permission state.

Phase A is therefore closed for the current overlay after focused tests, broad
regression, and monitor review. It remains on regression watch because TCML is
the architectural choke point for all later loop migration.

### 0.3 Current Phase B Status

Phase B is partially closed:

- Generic compatible Anthropic projection no longer sends flat string messages
  or `role:"tool"` entries for tool history. Assistant tool calls become
  `tool_use` blocks and tool results become user-side `tool_result` blocks.
- Generic compatible OpenAI projection now also uses typed serde JSON
  serialization for top-level request bodies and message content escaping.
- DeepSeek/Qwen incomplete streaming tool calls are drained at stream end,
  reported through incomplete-flush telemetry, and converted into model-readable
  TCML errors instead of disappearing.
- DeepSeek Anthropic tool-result error status now uses the private
  `__researchcode_tool_result_meta` tail envelope, not substring matching on
  ordinary stdout.
- Native Anthropic tool-result replay no longer injects non-standard
  message-level `reasoning_content`; thinking replay remains an Anthropic
  `thinking` content block. DeepSeek OpenAI `reasoning_content` replay remains
  intentionally native and must not be removed.
- Native DeepSeek Anthropic tool-result request bodies are now built with
  serde typed JSON instead of manual string concatenation, including complex
  tool input/result escaping. Malformed `input_json` is preserved as a sentinel
  only when replaying an error tool_result, so incomplete streamed tool calls
  can continue as model-readable recovery without allowing a non-error replay
  to silently carry invalid assistant tool input.
- Native DeepSeek Anthropic initial request bodies are now also assembled as a
  typed serde JSON object at the top level. Existing history projection remains
  unchanged for this slice, but model/system/tool/tool_choice/messages are
  serialized through `serde_json`, with a regression covering tool schemas plus
  quoted and multiline text.
- Initial DeepSeek OpenAI-compatible and Qwen/OpenAI-compatible request bodies
  are also top-level typed serde JSON now. Regressions assert that DeepSeek
  keeps native `thinking`, `reasoning_effort:"high"`, and stream usage
  telemetry options, while Qwen/OpenAI-compatible requests keep
  `reasoning_effort:"none"` and do not inherit DeepSeek-native fields.
- DeepSeek OpenAI-compatible and Qwen/OpenAI-compatible tool-result replay
  request bodies are now typed serde JSON as well. OpenAI wire semantics are
  preserved: `function.arguments` remains a string, DeepSeek-only
  `reasoning_content` replay remains native OpenAI-compatible behavior, and
  Qwen replay does not inherit DeepSeek `thinking` or stream usage options.
- Dual-protocol fallback cleanliness is now covered for both initial streaming
  attempts and native Anthropic tool-result continuation attempts: dirty
  structural events from the failed Anthropic 400 attempt do not reach the
  stream tool handler before the OpenAI retry succeeds.
- Successful streaming attempts now emit `model.stream.finish_reason` telemetry
  when the provider response body carries `finish_reason` or `stop_reason`.
  The event includes stream id, provider, protocol format, HTTP attempt,
  fallback count, status code, and finish reason.
- GUI incident canary passed on 2026-06-04:
  `node desktop/gui_three_round_smoke.mjs --incident-verify
  --incident-live-only --rounds=2 --provider=deepseek
  --autonomy-mode=conservative`. The run covered a file.write approval resume
  and a shell.command approval resume in one GUI session, both settling to
  `Completed`; artifacts are under
  `desktop/.gui-smoke-runs/2026-06-04T07-14-03-884Z/`.
- Incomplete-stream flush events now also include `provider_stop_reason` when
  the provider body exposes `finish_reason` or `stop_reason`.

The remaining Phase B work is no longer "fix the broken Anthropic compatible
path" as a broad statement. The precise remaining work is:

1. keep dual-protocol fallback cleanliness on regression watch for both initial
   and continuation attempts;
2. keep provider request serialization on regression watch and continue any
   remaining narrow cleanups found by focused scans;
3. ensure `finish_reason` and incomplete-stream metadata are observable enough
   for GUI/live canary debugging; **finish/stop reason telemetry is landed for
   successful streaming attempts with provider finish metadata.**
4. keep the deterministic regression bundle and GUI incident canary on the
   regression path; run the Argon-Agent-test long live canary when the user
   explicitly wants API-spend coverage.

### 0.4 Full Audit Read Planning Delta

After the full audit package was landed, the planning correction is not another
flat "fix every P0/P1" list. The audit files are now interpreted this way:

- `03_issue_matrix.md`, `04_doc39_conflict_matrix.md`,
  `04_architecture_review.md`, `06_remediation_grading.md`, and
  `08_final_audit_package.md` are evidence snapshots from the audited code
  baseline. They remain useful for root-cause and regression mapping, but they
  are no longer an executable queue by themselves.
- `05_red_team/*` is severity correction and architecture challenge material.
  It upgrades the importance of loop ownership, dirty stream events,
  convergence stop authority, GUI narration loss, shell interpreter denial, and
  evidence continuity.
- `09_corrected_architecture_remediation_plan.md` remains the architectural
  contract. This file is the current execution overlay over that contract.

The live queue therefore moves from Phase B-first to this order:

```text
Phase C loop-state/convergence spine
  + Phase D GUI lifecycle truth
  + Phase F deterministic/live harness gaps
  -> Phase E context spine
  -> Phase G AgentKernel ownership migration
```

Phase B stays active only as regression watch and live-canary proof. It should
not consume the next engineering slice unless a focused scan finds a concrete
remaining provider serializer or fallback-cleanliness regression.

### 0.5 Current Conflict Map

The remaining architecture conflicts that matter before AgentKernel migration
are:

| Conflict | Current handling |
|---|---|
| AgentKernel is still a hollow orchestrator | Keep as Phase G. Do not migrate before loop/GUI/context/harness truth gates are green. |
| Transition-string classifier looked like a finalizer reincarnation | Runtime authority removed for the current slice; keep telemetry-only guard and regression tests. |
| Loop state and convergence authority are still scattered | Phase C is now the next runtime architecture slice. |
| GUI can still infer completion from settlement/state refs | Phase D is next user-visible slice; especially approval and follow-up freeze. |
| Evidence and context continuity are short-term only | Phase E, after loop-state spine gives stable refs/terminal reasons. |
| Live long-task proof is incomplete | Phase F runs before claiming DeepSeek/Argon-Agent-test maturity. |

The explicitly closed items must stay closed by regression:

- no `agent.final_answer`, `visible_finalizer_answer`, or
  `model_continuation_skipped`;
- no workflow tool hiding as manifest policy;
- no concurrent read-only TCML bypass;
- no dirty structural stream events from failed provider attempts;
- no shell interpreter/destructive-program path that becomes approvable;
- no approval resume path that executes twice.

## 1. Planning Corrections

The original issue matrix is evidence, not the current queue. Several P0/P1
items have already been implemented in the current worktree and must be closed
or moved to regression watch instead of being reworked.

### Closed Or Regression-Watch Items

| Audit item | Current status | Evidence in code/tests |
|---|---|---|
| Dirty structural stream events before failed provider attempt | Closed for current slice | `failed_stream_attempt_does_not_call_structural_event_handler_before_fallback` in `scripts/run_phase1_agent_loop_regression.py` |
| Active turn stale release clobbers next turn | Closed for desktop/local API | `active_turns: HashMap<String, u64>`, `next_turn_generation`, `active_turn_generation_ignores_stale_release` |
| Pending native decision stale after cancel | Closed for native decision path | `RuntimePendingNativeDecision.turn_id`; cancel clears pending native decisions |
| Permission approval double execution | Closed for current native path | `double_native_permission_approval_does_not_execute_twice` |
| Classifier deny presented as approvable | Closed for permission gate | `classifier_approvable: false` maps to `PermissionResolution::Deny` |
| Shell interpreters and destructive programs not denied | Closed for current deny list slice | shell interpreter/system mutator deny tests and native hard-deny test |
| Mid-loop `EscalateToCodeEdit` manifest mutation | Closed | no production `EscalateToCodeEdit` branch remains in runtime code |
| Manifest hidden by exposure | Closed for manifest builder | `allow_tool_for_manifest` ignores exposure; stability tests added |
| Streaming write hidden by read-only exposure | Closed for stream routing | read-only exposure still routes write/edit/multi-edit to permission gate |
| Transition narration swallowed by finalizer replacement | Closed for runtime loop | transition narration is recorded as visible assistant text; transition detection is telemetry-only and does not continue the loop |
| Shell permission approval does not prove model continuation | Closed for deterministic facade coverage | `facade_shell_permission_resume_injects_tool_result_on_next_turn` verifies approved `shell.command` is replayed into the next provider request as a matching assistant `tool_call` + `role=tool` result |
| Concurrent read-only TCML bypass | Closed for current concurrent path | concurrent batches mediate with provider id, emit TCML telemetry, use canonical tool IDs/arguments, and reject non-read-only barriers back to serial permission flow |
| Generic compatible Anthropic flat projection | Closed for generic compatible builder | `anthropic_compatible_projects_tool_calls_and_results_as_content_blocks`; tool results without ids are locally rejected instead of fabricated |
| Generic compatible OpenAI manual serialization | Closed for generic compatible builder | `builds_openai_compatible_request_without_native_optimization` parses the request JSON and covers quoted/newline content |
| Incomplete streaming tool calls disappear | Closed for DeepSeek/Qwen stream processors | incomplete stream calls flush as TCML/model-readable parse errors with `deepseek.tool_call.incomplete_flushed` |
| Anthropic tool-result `is_error` substring detection | Closed for DeepSeek Anthropic continuation | private tail meta controls `is_error`; plain stdout containing `{"is_error":true}` remains ordinary content |
| Native Anthropic message-level `reasoning_content` injection | Closed for DeepSeek Anthropic tool-result replay | thinking replay uses `type:"thinking"` block and tests assert no `"reasoning_content"` in Anthropic request bodies |
| Malformed recovered Anthropic tool input breaks replay | Closed for recovery path | non-error malformed `tool_use.input_json` is rejected, but malformed input paired with an error `tool_result` is preserved as a sentinel so incomplete streaming recovery stays model-readable |
| Incomplete stream stop reason missing from flush telemetry | Closed for runtime events | `deepseek.tool_call.incomplete_flushed` includes `provider_stop_reason` when provider finish/stop metadata is present |
| Non-error convergence never stops | Partially closed | bounded convergence warnings now stop; same-tool-error threshold remains separate debt |

### Still Open Or Partially Open

| Priority | Item | Reason |
|---:|---|---|
| P0/P1 | Loop state and convergence authority remain fragmented | The runtime no longer uses transition text as loop authority, but scattered counters, warnings, terminal reasons, and evidence clearing still make long-task stop/continue behavior hard to reason about. This is the next runtime slice. |
| P1 | Evidence continuity remains fragmented | `last_tool_batch.clear()` and `evidence_ledger.clear()` still keep only immediate continuation evidence; acceptable short-term, but conflicts with long-task state spine and makes convergence/context work depend on fragile local state. |
| P1 | AgentKernel is still not the orchestrator | Kernel migration must happen after TCML/provider/context/GUI correctness gates. |
| P1 | Provider projection is mostly regression-watch, not the main queue | Generic compatible Anthropic/OpenAI and native DeepSeek/Qwen request builders are typed for the covered paths. Remaining provider work is narrow scanning plus live canary proof, not broad rewrite. |
| P1 | Dual-protocol fallback cleanliness needs live proof | Deterministic initial and continuation gates exist; Argon-Agent-test long live canary still needs to prove no dirty fallback state during real shell/plan/write pressure. |
| P1 | Incomplete streaming metadata GUI consumption is narrowed | Runtime event payloads expose provider finish/stop metadata; deterministic replay now proves output-limit stop metadata becomes a visible diagnostic progress item. Live canary coverage remains useful for provider-specific failure screenshots. |
| P1 | GUI lifecycle still needs event-truth hardening | Composer/status must derive from lifecycle events only; incremental markdown and large replay are not proven. |
| P1/P2 | Context spine is incomplete | L1 state object, reversible refs, and pointer page-back are not implemented as a system. |
| P2 | Permission and plan safety secondary paths | Plan text safety warnings, `artifact.export` content scan, permission submission rate limiting, and FileRead gate sensitivity remain open. |

## 2. Revised Execution Order

The execution order changes from "do all quick wins" to "finish the correctness
spine, then migrate ownership." The reason is simple: AgentKernel migration
before TCML/provider/permission truth would move bugs into a new shape.

### Phase A: TCML Sole-Path Enforcement

Goal: every model tool call, including concurrent read-only calls, goes through
TCML mediation before dispatch.

Status: closed for the current overlay; keep as regression guard.

Landed changes:

1. In the concurrent read-only builder, replaced raw `parse_tool_arguments` /
   `normalize_tool_id` with `mediate_tool_call_with_provider_id`.
2. Recorded TCML mediation events before concurrent execution.
3. Used canonical tool IDs for orchestration and evidence.
4. Preserved canonical argument JSON in evidence instead of `{}`.
5. Converted mediation errors into model-readable tool errors.
6. Kept concurrent execution read-only and prefix-ordered only; writes, shell,
   interactive, unknown-risk tools, and reads after a state-changing barrier
   remain in the serial permission-aware path.

Regression tests:

- concurrent alias -> canonical tool ID -> dispatch;
- concurrent argument repair/defaults are reflected in evidence;
- concurrent mediation error becomes a model-readable observation;
- every concurrent tool call has TCML telemetry before tool execution.

### Phase B: Provider Projection And Stream Completion

Goal: provider attempts cannot corrupt turn state, and malformed or incomplete
tool calls become explicit observations.

Required changes:

1. Rewrite compatible Anthropic projection with typed structured content blocks.
   **landed for the generic compatible provider request builder.**
2. Remove non-standard Anthropic `reasoning_content` fields where still present.
   **landed for native DeepSeek Anthropic tool-result replay: thinking replay
   is carried only as a typed `thinking` content block; OpenAI-compatible
   reasoning replay is unchanged.**
3. Parse tool-result `is_error` as structured JSON instead of substring checks.
   **landed for DeepSeek Anthropic continuation: only the private
   `__researchcode_tool_result_meta` tail envelope is honored; plain stdout
   containing `{"is_error":true}` no longer false-positives.**
4. Flush incomplete streaming tool calls at stream end as parse errors with
   finish reason telemetry. **model-readable parse-error path is landed;
   finish-reason/attempt telemetry is landed for successful streaming attempts
   that include provider finish metadata.**
5. Add request/response typed serialization tests for DeepSeek, Qwen, OpenAI
   compatible, and Anthropic compatible paths. **landed for the main
   DeepSeek/Qwen initial and tool-result replay request builders, generic
   compatible OpenAI/Anthropic projection, and fallback-cleanliness fixtures;
   keep on regression watch and continue narrow scans for projection helpers.**

Required tests:

- Anthropic-compatible tool-call history serializes as valid blocks; **landed for generic compatible provider request builder; orphan tool results now fail at local request construction instead of using fabricated `unknown` IDs**
- split DeepSeek tool-call deltas assemble correctly;
- incomplete tool args do not disappear; **landed for DeepSeek/Qwen stream processors and native loop model-readable continuation; native Anthropic replay preserves malformed input as a sentinel only when paired with an error result**
- failed Anthropic attempt cannot leave dirty tool state before OpenAI retry.

Current checkpoint:

- Landed: stream completion now drains incomplete streaming tool calls into
  completed TCML entries, records incomplete-flush telemetry, and routes them
  through the same model-readable error/result path as other malformed tool
  calls.
- Landed: generic compatible Anthropic request projection no longer emits
  flat `role:"tool"` messages; assistant tool calls become `tool_use` blocks
  and tool results become user-side `tool_result` blocks. Missing tool IDs now
  fail before HTTP rather than inventing `toolu_compatible_unknown`.
- Landed: generic compatible OpenAI request projection uses typed serde JSON
  construction, with tests parsing quoted/newline content back from the body.
- Landed: DeepSeek Anthropic `tool_result.is_error` is derived from a private
  structured tail envelope and stripped from the block content before provider
  replay. This closes the false-positive substring path.
- Landed: native DeepSeek Anthropic replay no longer injects non-standard
  message-level `reasoning_content`; tests assert the typed `thinking` block is
  preserved and `reasoning_content` is absent from that request.
- Landed: native DeepSeek Anthropic tool-result replay uses typed serde JSON
  construction for the request body, validates `tool_use.input_json`, and
  covers quoted/newline content plus malformed error-result replay in
  `live_model_request` tests.
- Landed: continuation fallback cleanliness now has a deterministic fixture:
  `continuation_stream_fallback_discards_failed_attempt_structural_events`.
- Landed: `model.stream.finish_reason` records provider finish/stop reason,
  protocol format, attempt, fallback count, and status code on successful
  streams.
- Landed: `deepseek.tool_call.incomplete_flushed` records
  `provider_stop_reason` when the successful stream body includes finish/stop
  metadata.
- Still open: Argon-Agent-test long live canary and plan-continuation GUI
  coverage; the focused GUI incident canary for file.write and shell.command
  approval resume is passing.

### Phase C: Loop State And Evidence Continuity

Goal: remove hidden loop authorities and make no-progress decisions traceable.

Required changes:

1. Introduce `NativeLoopState` or equivalent turn-state struct for iteration,
   provider attempt, tool batch staging, continuation reason, no-progress
   counters, and terminal reason.
2. Replace scattered `continue` decisions with named transitions.
3. Move `visible_text_looks_like_transition_statement` out of authority:
   it may label telemetry, but it must not be the sole reason to continue or
   complete.
4. Track repeated errors per tool/signature so unrelated successful reads do
   not reset write/edit failure plateaus.
5. Preserve enough cross-iteration evidence for convergence and context spine.

Required tests:

- interleaved success on one tool does not hide repeated failure on another;
- duplicate/no-new-evidence plateau stops structurally after bounded warnings;
- visible transition text is recorded but not used as terminal authority;
- every terminal state has a single typed reason.

Current checkpoint:

- Landed first spine slice: `NativeLoopState` now exists as a runtime-local
  ledger, not a second policy owner. It records the current iteration and emits
  `agent.loop_state.terminal` for covered terminal exits.
- Covered exits in this slice: preflight interruption/block/stop, context
  guard block, pending permission block, model-visible answer completion,
  rejected/empty visible answer failure, duplicate batch guard stop, tool
  iteration stop, max-iteration turn-budget stop, and loop-incomplete failure.
- Follow-up coverage landed: missing compaction summaries, missing prepared
  initial requests, compact-retry guard blocks, provider HTTP/transport failure
  stops, plan approval waiting, and ask-user waiting also emit
  `agent.loop_state.terminal`.
- Regression tests assert terminal ledger visibility for both ordinary
  `model_visible_answer` completion and max-iteration blocked stop, plus
  plan-approval, ask-user, and external-permission waiting states, while the
  transition detector remains telemetry-only.
- Landed semantic-unification slice: structured blocked stops now use a
  dedicated `stop_native_loop_with_structured_blocked` helper so legacy
  `agent.loop_stopped` events carry `status:"blocked"` for plateau, tool
  iteration, and max-iteration budget stops. Provider and answer-validation
  failures still use `status:"failed"`.
- Follow-up correction after monitor review: initial provider HTTP failures
  remain `status:"failed"`, tool-iteration and tool-budget stops are
  `status:"blocked"`, and `agent.loop_stopped.next_action` now distinguishes
  blocked stops from real failures. Tests now parse the specific
  `agent.loop_stopped` payload instead of relying on broad JSONL substring
  matches.
- GUI replay now treats `agent.loop_stopped.status == "blocked"` as a stopped
  turn/progress item, while real failed stops remain failed. This keeps blocked
  runtime control from surfacing as "execution failed" in the desktop lifecycle.
- Backend session-state cleanup landed for structured blocked stops:
  `stop_native_loop_with_structured_blocked` now records blocked turn summaries
  and transitions to `WaitingForUser`, while failed stops still transition to
  `Failed`.
- Terminal reason uniqueness landed: `NativeLoopState::record_terminal` is now
  first-write-wins for the authoritative `agent.loop_state.terminal` event.
  Later terminal attempts are emitted only as
  `agent.loop_state.terminal_duplicate_suppressed` diagnostics, so a turn has a
  single typed terminal reason even when a late path attempts to record another
  stop/failure. Regression coverage now asserts exactly one
  `agent.loop_state.terminal` for max-iteration blocked stops,
  telemetry-only visible transition answers, plan approval waiting, ask-user
  waiting, and external permission waiting. Direct state-level coverage
  `native_loop_state_records_only_one_terminal_reason` proves duplicate
  terminal attempts preserve the first reason and record the suppressed request.
- Event invariant coverage now enforces the same contract across replayed logs:
  `validate_event_invariants` rejects more than one authoritative
  `agent.loop_state.terminal` event per `loop_id` / turn, while allowing
  multiple turns in the same session log and allowing the diagnostic
  `agent.loop_state.terminal_duplicate_suppressed` event. This moves terminal
  uniqueness from per-fixture assertions into the shared event-log contract
  without misclassifying multi-turn session logs.
- Event invariant coverage now also rejects native-loop terminal session states
  that have no authoritative terminal ledger event on the same turn. Native
  `turn.route.classified` events now include `turn_id`, and
  `validate_event_invariants` tracks terminal requirements per `turn_id` /
  `loop_id` instead of using a global "any terminal exists" check. In-progress
  turns may remain without a terminal event, but once that turn emits a
  terminal-ish `session.state_changed` or loop-stopped event,
  `agent.loop_state.terminal` is required for the same id. This makes the
  terminal ledger a replay contract instead of a best-effort side channel.
  Focused coverage includes the two-turn regression where turn 1 has a
  terminal ledger and turn 2 is missing one, plus the inverse case where turn 2
  is still executing and must not be rejected. Tests: `cargo test -p
  researchcode-runtime event_invariants` and `cargo test -p researchcode-runtime
  native_agent_loop`.
- Duplicate/non-progress tool plateau convergence now has a bounded stop path
  in `ToolProgressState`: duplicate observations and non-progress iterations
  soft-warn at 3 consecutive iterations and stop at 6, while any new evidence
  resets the progress counters. `NativeLoopTurnController` records the stop as
  `agent.loop_plateau_stopped`, and the native loop already maps controller
  stop actions into structured blocked loop terminals instead of completion.
  Focused coverage: `cargo test -p researchcode-runtime
  agent_kernel::turn_state` and `cargo test -p researchcode-runtime
  agent_kernel::turn_controller`.
- Still open: continue replacing scattered ad hoc `continue`/stop handling with
  named state updates and audit remaining early returns for terminal ledger
  coverage.

### Phase D: GUI Lifecycle And Transcript Truth

Goal: the GUI follows runtime lifecycle events and never infers completion from
tool-card settlement or partial text.

Required changes:

1. Composer state derives from:
   - `agent.turn.completed`
   - `agent.turn.failed`
   - `agent.turn.blocked`
   - `agent.turn.interrupted`
   - `agent.turn.awaiting_permission`
   - `agent.turn.awaiting_plan`
2. Reset stream refs by generation, not timers or booleans.
3. Preserve pre-tool, inter-tool, and post-tool narration as separate ordered
   transcript blocks.
4. Add incremental markdown rendering for streaming text without reparsing the
   full transcript per frame.
5. Add capped transcript/dedupe memory with replay-safe behavior.

Required tests:

- pre-tool/inter-tool/post-tool text remains visible;
- approval modal approve -> runtime resumes -> composer does not freeze;
- stopping a turn releases composer only after interruption ack;
- 1000+ event replay remains responsive.

Current checkpoint:

- Landed first lifecycle-truth slice: desktop snapshot polling now preserves a
  `stopped` GUI state when a prior runtime event already classified the turn as
  a structured blocked stop. A later legacy `Failed` snapshot no longer
  overwrites that event-derived stopped state, while new `Executing` snapshots
  still reopen the run and real failed events still mark the run failed.
- Follow-up backend cleanup means structured blocked-stop snapshots now surface
  as `WaitingForUser`, which the desktop maps to `stopped` instead of falling
  back to `idle`.
- Follow-up submission after a structured blocked stop is reopened correctly:
  `begin_interactive_turn` now treats `WaitingForUser` like other terminal-ish
  user-facing states and transitions back to `Executing`, and the desktop
  follow-up gate treats `WaitingForUser` as ready for a new turn.
- Snapshot/event ordering hardening landed: desktop polling now applies a
  runtime snapshot state only when `snapshot.event_count` exactly matches the
  GUI event cursor. A stale snapshot can no longer overwrite a newer
  event-derived run status, and a leading snapshot can no longer represent
  terminal events the GUI has not consumed yet. `session.state_changed` remains
  the GUI lifecycle authority after push/poll interleaving.
- Verification: `desktop/test_runtime_event_replay.mjs` covers the snapshot
  preservation rule and `npm run build` for `desktop/` passes. Runtime
  regression covers `waiting_for_user_session_can_start_new_interactive_turn`
  and the Phase 1 agent-loop pack.
- Still open: plan approval continuation live canary and stronger GUI
  long-task stress coverage.

### Phase E: Context Spine And Reversible Compaction

Goal: implement the L0-L4 memory model without making Flash compactor a
first-wave blocker.

Required changes:

1. Add pinned L1 state object:
   - overall goal;
   - current subgoal;
   - confirmed facts with refs;
   - decisions;
   - open questions;
   - touched resources;
   - next expected action.
2. Keep L0/L1 pinned in every model request.
3. Keep recent action-observation pairs raw in L2 only.
4. Fold older observations into L3 summaries with `ref://` pointers.
5. Keep raw events/tool output in L4 event archive and page back by pointer.
6. Preserve the latest required DeepSeek reasoning for adjacent replay.
7. Treat separate Flash compactor as a later optimization unless eval proves
   in-process summaries fail.

Required tests:

- compaction above threshold preserves L1;
- old observation can be paged back by pointer;
- DeepSeek reasoning replay survives one compaction boundary;
- compaction cannot inject fabricated final answers.

2026-06-04 checkpoint:

- Landed first L1 spine slice under `AgentKernel`: `ContextSpineState` extracts
  a pinned state object from the replayable event log with overall goal,
  current subgoal, confirmed facts, open questions, decisions, resources, next
  steps, and `ref://event/<sequence>` pointers.
- Native preflight compaction now carries the spine in `NativeContextGuardReport`
  and injects `[pinned-context-spine]` into compacted DeepSeek requests before
  the lossy L3 summary. `context.compaction.completed` also records a `spine`
  field for replay/GUI inspection.
- Compactor output now includes both `[pinned-context-spine]` and
  `[compacted-context]`, so compressed history no longer relies on summary text
  as the only continuity carrier.
- Verification landed:
  `context_spine_extracts_pinned_state_with_reversible_refs`,
  `pages_raw_event_back_from_reversible_ref`,
  `compaction_preserves_recent_turns`,
  `native_agent_loop_v2_long_context_compaction_drops_next_prompt_tokens`,
  `native_agent_loop_v2_context_compaction_folds_reasoning_replay`,
  `desktop/test_runtime_event_replay.mjs`, and
  `scripts/run_phase1_agent_loop_regression.py`.
- `EventLog::page_ref("ref://event/<sequence>")` now provides the first L4
  raw-event page-back API. Still open: wiring page-back into model/GUI
  retrieval flows, stronger L2 recent action-observation window policy,
  observation-to-L3 lifecycle tests, and GUI visibility for compaction/spine
  state.
- Monitor correction landed: L1 confirmed facts now require trusted successful
  tool-result evidence (`tool.result_recorded` paired with `ok=true` by tool
  call id). Ledger/outcome events such as `agent.tool.completed` only feed the
  outcome map; they are not fact sources. Failed, unconfirmed, recovery-like,
  or synthetic tool results stay in non-authoritative `observations` instead of
  `confirmed_facts`.
- Monitor correction landed: the model-facing spine is rendered as a fenced JSON
  data block, with labels treated as quoted runtime data rather than prompt
  instructions. `context.compaction.completed` now records both human-readable
  `spine` markdown and structured `spine_json`.
- Negative coverage now includes failed tool results not becoming confirmed
  facts, final-answer-like text not becoming a pinned decision, and markdown
  heading injection staying quoted inside the JSON data block. Follow-up
  coverage also proves `agent.tool.completed(ok=true)` alone cannot create a
  confirmed fact and that paired success confirms only the result event.
- Boundary note: `EventLog::page_ref` is an internal L4 primitive only. Exposing
  it to model/GUI retrieval still requires session/task scoping, redaction, and
  permission checks.
- L2/L3 slice landed: compaction now preserves recent raw turns by turn window
  instead of a fixed event-count approximation. Older `tool.result_recorded`
  observations are folded into L3 `latest_tool_evidence` entries with
  `ref://event/<sequence>` pointers, and
  `compaction_keeps_recent_turn_window_and_folds_old_observations_with_refs`
  proves those refs can page back to raw L4 events.
- Monitor correction landed: `projection.boundary_event` now points to the
  final event of the last compacted turn, not an event index derived from the
  compacted turn count. L3 observation summaries exclude ledger-only
  `agent.tool.completed` events; those can feed progress/outcome notes but not
  observation evidence.
- L4 page-back is now available through the product runtime boundary, not only
  as an internal `EventLog` helper. `RuntimeFacade::page_context_ref(session_id,
  "ref://event/<sequence>")` pages a raw event back from the current session
  archive, verifies the event belongs to the same session and task, and returns
  a projected model/GUI-readable message plus event metadata. Invalid schemes,
  unknown refs, and unknown sessions fail at the facade boundary. Focused
  coverage: `facade_pages_context_ref_with_session_and_task_scope` and
  `facade_rejects_context_ref_page_back_for_mismatched_session_or_task_event`
  inside `cargo test -p researchcode-runtime runtime_facade`, plus existing
  `EventLog::page_ref` and compactor ref tests.
- Remaining Phase E gaps are now narrower: GUI/model retrieval flows still need
  a user-facing affordance for when to page refs back, and compaction/spine
  visibility in the desktop can be made richer. The correctness spine itself
  has L1 state, L2 recent-turn preservation, L3 ref summaries, and L4 scoped
  page-back.
- Desktop compaction visibility now consumes the structured runtime payload
  instead of showing only a generic completed chip. `context.compaction.completed`
  updates the context-pressure label with before/after token estimates when
  present, and the progress detail reports L1 spine counts plus `ref://event`
  reference counts from the summary. The parser accepts both object and string
  forms of `spine_json`, keeping the GUI tolerant of runtime serialization
  shape. Verification: `node desktop/test_runtime_event_replay.mjs`,
  `node desktop/test_desktop_polish_contract.mjs`, and `npm --prefix desktop
  run build`.

### Phase F: Deterministic Harness And Live Canary

Goal: make real DeepSeek pressure testing a drift detector, not the primary
correctness proof.

Required deterministic fixtures:

1. shell approval -> execute -> model continuation; **landed for RuntimeFacade/provider-request projection and GUI live canary**
2. plan approval -> synthetic artifact -> model continuation; **landed for GUI mock canary**
3. TCML alias/schema/repair/default/permission/dispatch chain;
4. Anthropic 400 fallback with no dirty structural events;
5. incomplete tool call at stream end;
6. cancel with pending decision then new turn;
7. active-turn generation stale release;
8. large tool result and 1000+ event GUI replay;
9. subagent lifecycle replay, even before full subagent execution.

Live DeepSeek canary requirements:

- use the Anthropic-compatible protocol path unless explicitly testing OpenAI;
- run in `Argon-Agent-test`;
- include plan approval, shell approval, file write/edit, read-back, test run,
  and follow-up question;
- assert no `agent.final_answer`, `visible_finalizer`, or
  `model_continuation_skipped`;
- assert no premature GUI `Completed`;
- assert no unresolved active turn after terminal event.

2026-06-04 checkpoint:

- Deterministic GUI full-stack canary passed:
  `node desktop/gui_full_stack_regression.mjs --rounds=6 --provider=deepseek`.
  Evidence: `desktop/.gui-smoke-runs/full-stack-2026-06-04T11-11-48-808Z/report.json`
  with 6 completed turns, 46 requested/completed tool events, plan approval,
  shell permission approval, subagent artifact refs, 0 duplicate event ids, and
  no console/request blocking failures.
- Deterministic GUI plan-approval canary passed:
  `node desktop/gui_plan_approval_smoke.mjs`. Evidence:
  `desktop/.gui-smoke-runs/plan-approval-2026-06-04T11-11-49-632Z/report.json`
  with approve decision and `dialog_cleared: true`.
- Deterministic GUI conversation-quality canary passed:
  `node desktop/gui_conversation_quality_smoke.mjs`. Evidence:
  `desktop/.gui-smoke-runs/conversation-quality-2026-06-04T11-11-57-578Z/report.json`
  with inter-tool text, long stream, final supplement, interrupt notice, and
  follow-up after interrupt all visible.
- Strengthened Stop lifecycle gate:
  `desktop/gui_conversation_quality_smoke.mjs` now delays the mock
  `/runtime/interrupt-session` acknowledgement and asserts that the GUI remains
  `running` before the runtime ack, then releases to `stopped` only after the
  runtime emits `runtime.turn_cancel_requested` / `session.state_changed`.
  Evidence:
  `desktop/.gui-smoke-runs/conversation-quality-2026-06-05T00-02-10-781Z/report.json`
  with `stopHeldRunningBeforeAck:true`, `stopReleasedAfterAck:true`,
  `debugBeforeInterruptAck.run_status:"running"`,
  `debugAfterInterruptAck.run_status:"stopped"`, and follow-up submission
  succeeding after the interruption.
- Strengthened approval-resume lifecycle gate:
  `desktop/gui_permission_longtask_smoke.mjs` now asserts that after approving a
  pending `shell.command`, the GUI has consumed
  `runtime.permission_resume.completed` and still remains `running` until the
  terminal lifecycle, then releases to `completed` and accepts an immediate
  follow-up turn. Evidence:
  `desktop/.gui-smoke-runs/permission-longtask-2026-06-05T00-15-07-420Z/report.json`
  with `approval_resume_held_running_before_terminal:true`,
  `approval_resume_released_after_terminal:true`,
  `followup_submitted_after_approval_resume:true`,
  `followup_done_visible:true`, 49/49 requested/completed tool calls, and
  `resume_completed_sequence:84`,
  `debug_after_resume_completed_before_terminal.cursor:87`,
  `debug_after_resume_completed_before_terminal.run_status:"running"`, and
  `terminal_event_at_resume_checkpoint:null`.
- Strengthened plan-approval lifecycle gate:
  `desktop/gui_plan_approval_smoke.mjs` now delays terminal completion after
  plan approval, emits a real post-approval work segment, and asserts that the
  GUI has consumed `runtime.plan_approval.model_continued` while still
  `running`. It then requires terminal release to `completed` and an immediate
  follow-up response. Evidence:
  `desktop/.gui-smoke-runs/plan-approval-2026-06-05T00-20-24-522Z/report.json`
  with `plan_continued_sequence:8`,
  `debug_after_plan_continued_before_terminal.cursor:10`,
  `debug_after_plan_continued_before_terminal.run_status:"running"`,
  `terminal_event_at_plan_continued_checkpoint:null`,
  `plan_resume_released_after_terminal:true`,
  `followup_submitted_after_plan_resume:true`, and 6/6 requested/completed tool
  calls.
- Provider stream stop metadata is now GUI-visible in deterministic replay:
  `desktop/src/runtime/runtimeEventReducer.ts` returns a diagnostic progress
  item for non-empty `model.stream_completed.stop_reason` /
  `finish_reason` / provider alias metadata, and
  `desktop/src/hooks/useRuntimeEventApplication.ts` inserts that item into the
  progress timeline. Output-limit reasons such as `max_tokens` are categorized
  as recovery and do not settle the turn as a false completion. Evidence:
  `node desktop/test_runtime_event_replay.mjs` asserts
  `模型输出达到上限: max_tokens` with `eventType: "model.stream_completed"`; it
  also locks `finish_reason`, `provider_stop_reason`,
  `provider_finish_reason`, and ordinary `end_turn` no-noise behavior.
  `node desktop/test_desktop_polish_contract.mjs` locks the reducer/hook
  contract.
- Live DeepSeek incident canary passed:
  `node desktop/gui_three_round_smoke.mjs --incident-verify --incident-live-only --rounds=2 --provider=deepseek --autonomy-mode=conservative`.
  Evidence:
  `desktop/.gui-smoke-runs/2026-06-04T11-24-00-291Z/report.json` and
  `incident_live_assessment.json`; file.write and shell.command approvals both
  resumed to `Completed`, permission requests/decisions were 2/2, unresolved
  permission requests/stale permission errors/shell permission failures were
  empty, failures/warnings were empty, and the refreshed report had no
  `runtime_http_errors`, no `console_errors`, and no request failures. Harness
  reporting now separates blocking runtime HTTP errors, ignored runtime HTTP
  errors, blocking console errors, and ignored shutdown noise so a browser
  `400 (Bad Request)` cannot remain unexplained at the report top level.
- Argon long-task harness safety/coverage was strengthened:
  `desktop/gui_argon_longtask_stress.mjs` now requires
  `--allow-external-workspace-provider` before sending the real
  `Argon-Agent-test` workspace to DeepSeek, and offers `--synthetic-workspace`
  to generate a non-private VoiceNote-shaped fixture for pipeline pressure
  testing. The report records synthetic-vs-real mode, runtime HTTP error
  classification, console classification, expected test-file write evidence,
  and read-back coverage.
- Synthetic Argon long-task run exposed a real convergence gap before the fix:
  `node desktop/gui_argon_longtask_stress.mjs --rounds=4 --provider=deepseek
  --synthetic-workspace` produced
  `desktop/.gui-smoke-runs/argon-longtask-2026-06-04T11-28-57-939Z/report.json`.
  After plan approval resumed correctly, the model repeatedly requested
  `file.read` with invalid `{}` arguments; the run reached 13,914 events,
  129 completed tool records, 87 `SCHEMA_VALIDATION_FAILED` tool errors, and
  no expected `ArgonGuiLongTaskProbeTests.swift` write before manual cancel.
  This was not a shell/plan approval failure; it was a contract-error recovery
  plateau that the previous mixed-success reset logic failed to stop.
- Landed convergence fix: `ToolProgressState` now tracks repeated
  `SCHEMA_VALIDATION_FAILED` contract errors separately from ordinary
  path/tool errors, and the DeepSeek streaming tool-result branch now feeds
  completed streamed batches through the same `observe_completed_tool_iteration`
  controller path as serial/concurrent tools. Repeated invalid tool arguments
  therefore stop with structured `tool_contract_error_plateau` even when the
  same iteration also contains successful or cached reads. Focused checks passed:
  `cargo test -p researchcode-runtime
  contract_error_plateau_is_tracked_separately_from_success_progress`,
  `cargo test -p researchcode-runtime
  controller_stops_repeated_schema_validation_errors_even_with_successes`,
  `cargo test -p researchcode-runtime
  controller_mixed_success_same_error_resets_streak_without_hard_stop`, and
  `cargo test -p researchcode-runtime
  native_agent_loop_v2_incomplete_streamed_tool_call_becomes_model_readable_error`.
- Synthetic Argon long-task rerun after wiring streaming convergence produced
  `desktop/.gui-smoke-runs/argon-longtask-2026-06-04T13-16-01-998Z/report.json`.
  It still failed the requested write/shell coverage because the model kept
  issuing invalid `file.read` calls, but the runtime now stopped structurally
  instead of running unbounded: 5,018 events, 45 tool completions, terminal
  `agent.loop_state.terminal` reason `tool_contract_error_plateau`, and GUI
  text `本轮已停止: tool_contract_error_plateau`. This proves turn release and
  structured failure surfacing; it does not yet prove the full Argon write/shell
  long-task path.
- Landed plan-approval continuation fix: the local API approval resume prompt
  now preserves the original user task instead of replacing it with only
  `The plan was approved...`; `TurnRouter` also prioritizes test/write/fix/
  implement semantics over review words. This keeps the post-plan turn on the
  original RunTests/CodeEdit route and prevents tool exposure from degrading to
  read-only after approval. Focused checks passed:
  `cargo test -p researchcode-runtime
  approved_plan_resume_prompt_preserves_original_user_task`,
  `cargo test -p researchcode-runtime
  permission_resume_prompt_uses_last_user_task_without_preamble`, and
  `cargo test -p researchcode-runtime agent_kernel::turn_router::tests`.
- Landed Anthropic-compatible streaming assembler fix: streamed native tool
  calls now accumulate `ToolCallStarted` and `ToolCallArgumentsDelta` without
  execution and only assemble after `ToolCallFinished` / content block stop.
  This removes the DeepSeek `{}` `file.read` storm caused by executing a
  tool-use block before its `input_json_delta` fragments arrived. Focused
  checks passed:
  `cargo test -p researchcode-runtime
  streaming_tool_assembler_completes_split_json_arguments`,
  `cargo test -p researchcode-runtime
  stream_processor_assembles_tool_call_arguments`,
  `cargo test -p researchcode-runtime
  stream_processor_ingests_sse_chunks_into_state_events`, and
  `cargo test -p researchcode-runtime
  pipeline_accumulates_streaming_tool_calls`.
- Landed GUI/event-stream resilience fix: event delta cursors beyond the
  current replayed event count now resync to the current tail and return an
  empty delta instead of HTTP 400. This prevents polling cursor drift from
  surfacing as a GUI/runtime error after long replay/merge sessions. Focused
  check passed:
  `cargo test -p researchcode-runtime
  facade_streams_incremental_event_deltas_by_cursor`.
- Synthetic Argon long-task final proof passed:
  `node desktop/gui_argon_longtask_stress.mjs --rounds=4 --provider=deepseek
  --synthetic-workspace`. Evidence:
  `desktop/.gui-smoke-runs/argon-longtask-2026-06-04T13-56-24-763Z/report.json`.
  The run completed 4 GUI turns, 10 model calls, 1,466 streaming deltas,
  10 completed tool records, plan approval, 2 permission requests/decisions,
  `file.write`, `shell.command`, and read-back of
  `VoiceNote/Tests/VoiceNoteTests/ArgonGuiLongTaskProbeTests.swift`; the
  expected XCTest probe file existed with 54 lines, and blocking request,
  runtime HTTP, and console errors were empty.
- `desktop/gui_argon_longtask_stress.mjs` now treats a structured
  `agent.loop_stopped` / `agent.loop_plateau_stopped` / terminal loop-state
  reason as an immediate coverage failure, so the harness no longer waits for
  shell/permission evidence after the runtime has already released the turn as
  blocked.
- Still open only by privacy boundary: the real `Argon-Agent-test` live
  workspace canary. Running it requires explicit user acceptance that copied
  workspace contents may be sent to DeepSeek via
  `--allow-external-workspace-provider`; the synthetic fixture now proves the
  runtime/GUI pipeline path without sending private workspace files.
- Landed privacy dry-run contract for the real canary:
  `node desktop/gui_argon_longtask_stress.mjs --privacy-dry-run
  --provider=deepseek` now writes a report without starting a browser, runtime
  server, model call, workspace copy, or provider request. Evidence:
  `desktop/.gui-smoke-runs/argon-longtask-2026-06-04T14-48-15-576Z/report.json`.
  The default report now redacts the real `Argon-Agent-test` manifest to
  stable canary input slots, stores prompt hashes plus redacted previews rather
  than full prompt text, redacts path-bearing coverage labels, and records the
  explicit external-provider consent boundary with
  `may_send_workspace_contents:false`. `--include-workspace-manifest` is an
  explicit opt-in for non-redacted manifest reporting. This dry-run is now part
  of `scripts/check_all.py`, so the live canary contract cannot silently drift
  while the privacy boundary remains unaccepted.
- Landed deterministic subagent lifecycle replay gate:
  `desktop/test_runtime_event_replay.mjs` now replays
  `subagent.spawned`, `subagent.message_sent`,
  `subagent.model_turn_started`, `subagent.tool_completed`,
  `subagent.summary_recorded`, `subagent.completed`,
  `subagent.tool_blocked`, `subagent.failed`, and
  `subagent.cancelled` events. The GUI reducer must preserve these as
  `subagent` progress entries with stable subagent ids and terminal statuses,
  including failed/cancelled detail strings that identify the subagent id,
  while keeping the parent session lifecycle independent. Focused verification
  passed:
  `node desktop/test_runtime_event_replay.mjs`.
- Landed deterministic large-output and 1000+ GUI replay gate:
  `desktop/gui_toolstorm_latency_smoke.mjs` now defaults to 250 completed
  tool calls, requires at least 1000 runtime events, injects a 24KB
  `tool.result_recorded` payload, asserts exact requested/completed tool
  counts, rejects duplicate event ids, requires the GUI-side debug cursor to
  reach the runtime event tail, and keeps input latency bounded during replay.
  Evidence:
  `desktop/.gui-smoke-runs/toolstorm-2026-06-04T23-40-57-136Z/report.json`
  with `event_count:1021`, `tool_requested/tool_completed:250/250`,
  `large_output_events:1`, `large_output_bytes:24000`, `duplicate_event_ids:0`,
  `gui_debug.cursor:1021`, and `input_latency.p95_ms:17.8` under the 120ms
  budget.

### Phase G: AgentKernel Ownership Migration

Goal: move orchestration out of monolithic `native_agent_loop.rs` after the
truth layers above are green.

Required changes:

1. Make `AgentKernel::run_turn` call its services directly.
2. Keep `NativeProfile` responsible only for DeepSeek/Qwen request, stream,
   parser, and profile behavior.
3. Unify initial and continuation turns into one iteration pipeline.
4. Replace inline JSON event construction with typed event builders.
5. Remove duplicated resume paths after compatibility shims are proven.

Acceptance:

- RuntimeFacade delegates loop policy to AgentKernel only;
- all model tool calls pass through TCML and PermissionGate;
- replay/event invariant suite passes;
- no old finalizer/tool-hiding/hard-cap completion authority returns.

## 3. Explicit Deferrals

These remain important but must not block the correction spine:

- full MCP dynamic registration;
- full OpenClaudeCode-grade subagent spawning;
- ripgrep feature parity;
- streaming tool execution overlap;
- full provider trait cleanup;
- provider fallback beyond the current DeepSeek/Qwen/compatible needs;
- hook ecosystem expansion;
- full async transport migration.

## 4. Current Next Slice

The immediate checkpoint is no longer Phase A. Phase A is closed for the
current overlay and should only be watched by regression.

The next implementation queue is:

1. **Phase C loop-state spine:** introduce an explicit `NativeLoopState` (or
   equivalent) and make terminal/continuation reasons named, observable, and
   testable. Transition-text detection is already telemetry-only; the remaining
   work is consolidating counters, evidence, convergence warnings, and terminal
   decisions into a traceable state object.
2. **Phase D GUI lifecycle:** prove the desktop composer/status follows turn
   lifecycle events and not tool-card settlement. This is the remaining risk
   behind "approved shell then GUI shows completed/freezes", "next question does
   not respond", and text appearing only after Stop.
3. **Phase F harness gaps:** add or strengthen deterministic plan approval,
   large replay, subagent replay, and Argon-Agent-test live canary coverage.
   Real API pressure is validation, not a substitute for deterministic gates.
4. **Phase E context spine:** implement L1 pinned state and reversible
   `ref://` observation pointers after the loop has stable state/terminal
   reasons to reference.

Phase B is now regression-watch plus live proof. Do **not** spend the next slice
on provider cleanup unless a concrete focused scan finds an uncovered serializer
or failed-attempt dirty-event path.

After Phase C/D/F, do **not** jump directly to AgentKernel migration unless the
context spine decision is explicitly deferred. The future migration should move
a coherent state machine, not the current scattered loop authorities.

### 4.1 Next Slice Acceptance Checklist

The next implementation slice is complete only when these concrete gates pass.

#### Phase C: Loop-state spine

Required code shape:

- A named runtime state object, `NativeLoopState` or equivalent, owns iteration
  number, provider-attempt state, current tool batch, continuation reason,
  no-progress counters, evidence summary, and terminal reason.
- Existing scattered loop exits route through named transition helpers or
  typed terminal reasons.
- `visible_text_looks_like_transition_statement` remains telemetry-only. It may
  label text, but it cannot be a stop/continue authority.
- Repeated-error tracking is keyed by tool/signature. Success from unrelated
  tools must not reset a failing write/edit/shell signature.

Required tests:

- visible transition text is recorded and does not trigger an extra hidden
  model iteration;
- duplicate/no-new-evidence plateau stops structurally after bounded warnings;
- interleaved successful reads do not mask repeated write/edit failures;
- every terminal event carries one typed terminal reason.

Stop condition:

- stop and report if implementing this requires reintroducing finalizer tools,
  tool disabling, or a hard tool-call cap as normal control.

#### Phase D: GUI lifecycle truth

Required code shape:

- Composer enabled/disabled state follows runtime lifecycle events, not
  transcript text, tool-card completion, or debounce timers.
- Stream/transcript refs are scoped by session/turn/stream generation.
- Pre-tool, inter-tool, and post-tool assistant text are represented as ordered
  transcript blocks.
- Recoverable tool failures are visible enough for diagnosis and cannot make a
  turn look successfully completed by themselves.

Required tests:

- approval modal approve -> runtime resumes -> composer remains correctly
  locked until terminal lifecycle event, then releases;
- approval resume completes -> composer releases -> immediate follow-up user
  turn starts and streams normally;
- Stop releases composer only after runtime interruption acknowledgement;
- pre-tool, inter-tool, and post-tool text survive reducer replay;
- 1000+ event replay stays bounded and responsive.

Stop condition:

- stop and report if GUI state can only be made correct by inventing client-side
  completion heuristics that disagree with runtime lifecycle events.

#### Phase F: Harness and live proof

Required deterministic gates:

- plan approval -> synthetic artifact -> provider continuation;
- shell approval -> execution -> provider continuation remains green;
- Anthropic fallback emits no failed-attempt structural tool events;
- incomplete streamed tool call produces model-readable error and telemetry;
- cancel with pending decision then new user turn;
- active-turn stale release cannot clobber a newer turn;
- large output and 1000+ GUI replay;
- subagent lifecycle replay fixture, even if real subagent execution remains a
  later capability.

Required live gate:

- Argon-Agent-test long task over the Anthropic-compatible path, covering plan
  approval, shell approval, file write/edit, read-back, test run, and follow-up
  question.

Stop condition:

- stop and report if deterministic gates pass but live canary fails in a way
  that requires changing DeepSeek/Qwen native prompt, parser, context policy, or
  tool policy. That is a native-optimization decision and needs explicit
  rollback/eval notes.

### Current Checkpoint After Shell Permission Fixture

The deterministic shell permission continuation gap is narrowed from
"untested" to "covered at RuntimeFacade/provider-request projection." The
remaining risk is GUI/Tauri lifecycle: approval modal decisions still need to
be exercised in a real desktop session to prove composer state and transcript
state do not infer premature completion.

### Current Checkpoint After Provider Projection Slices

Generic compatible Anthropic projection, incomplete-stream flush, structured
DeepSeek Anthropic tool-result error status, native Anthropic
`reasoning_content` cleanup, typed native Anthropic tool-result request
construction, and continuation dirty-fallback protection are landed. Remaining
provider work is regression-watch, narrow serializer scans, and live canary
proof. It is no longer the primary blocker before Phase C/D work.

Do not begin AgentKernel migration until Phase A-F gates are green.

### Current Checkpoint After AgentKernel Service Ownership Slice

`AgentKernel::run_turn` now passes its request-scoped service graph into the
native DeepSeek/Qwen loop through a `with_kernel` entrypoint. The older direct
native loop entrypoints remain compatibility shims and still materialize a
request-scoped kernel internally, but the RuntimeFacade -> AgentKernel path no
longer creates a second `AgentKernel::for_request` service graph inside the
loop.

This matters for shell/file approval recovery because permission state,
convergence state, context management, and future kernel services must be owned
by the caller-visible kernel, not reconstructed from request fields after the
facade has already selected a kernel. A behavior-level regression test proves
that a request with `PermissionMode::Default` obeys the caller-owned
`PermissionGate` mode when running through `AgentKernel::run_turn`; if the loop
falls back to request-owned permission mode, the test blocks on permission
instead of executing the scripted write.

Additional shell-command regressions cover the GUI failure class where a shell
approval resumes but the turn stalls: one test proves a caller-owned
`PermissionGate` controls shell execution mode, and another proves an already
provided shell permission decision is consumed through the AgentKernel entrypoint
and continues to command execution instead of returning a pending approval.

The obsolete private `run_native_agent_loop_v2_deepseek_resume` helper was
removed instead of migrated. It had no callers, carried an independent
`PermissionMode::Default` gate, and would have been a second resume policy owner
if reattached later. The live/facade resume paths remain covered by existing
native pending package, external resume, and facade shell permission resume
tests. Its orphan `NativeAgentLoopV2ResumeRequest` DTO was also removed, and the
remaining native permission-gate helper was restricted to tests.

Focused gates passed:

- `cargo test -p researchcode-runtime agent_kernel::kernel`
- `cargo test -p researchcode-runtime kernel_run_turn_ -- --nocapture`
- `cargo test -p researchcode-runtime native_agent_loop_v2_fastauto_write_executes_file_write`
- `cargo test -p researchcode-runtime native_agent_loop`
- `cargo test -p researchcode-runtime runtime_facade`

### Current Checkpoint After Permission Resume Execution Ownership Slice

Desktop/local API permission approval still spans two phases: execute the
approved pending tool, then start the next model turn from recorded evidence.
The first phase is now routed through an `AgentKernel` permission-resume service
instead of letting `RuntimeFacade` directly own `execute_tool` and resume error
normalization. `RuntimeFacade` remains responsible for session locking,
permission decision events, event artifacts, state transitions, and scheduling
the continuation turn.

This is intentionally a boundary-preserving intermediate step. It avoids a
large lock-held refactor while moving the tool execution authority toward the
same kernel service graph used by normal native turns. The GUI-visible event
contract is unchanged: `runtime.permission_resume.started`,
`runtime.permission_resume.tool_executed`, and
`runtime.permission_resume.completed` still describe the approval recovery.

Focused regressions prove the failure class seen in the GUI:

- approved native shell permission executes exactly once and reports
  `model_continuation_required=true`;
- the following model turn receives the approved shell call and matching tool
  result in provider history;
- caller-owned AgentKernel permission mode still controls shell/file execution.

Focused gates passed:

- `cargo test -p researchcode-runtime facade_permission_decision_executes_pending_native_tool_with_outcome -- --nocapture`
- `cargo test -p researchcode-runtime facade_shell_permission_resume_injects_tool_result_on_next_turn -- --nocapture`
- `cargo test -p researchcode-runtime kernel_run_turn_ -- --nocapture`
- `cargo test -p researchcode-runtime runtime_facade -- --nocapture`
- `cargo test -p researchcode-runtime native_agent_loop -- --nocapture`
- `node desktop/test_runtime_event_replay.mjs`
- `node desktop/test_desktop_polish_contract.mjs`
- `npm --prefix desktop run build`

### Current Checkpoint After Approval Active-Turn Boundary Slice

Desktop/Tauri and local API active-turn gates now treat approval waiting states
as resumable boundaries. If a session has already reached
`WaitingForToolApproval` or `WaitingForPlanApproval` while an older background
turn generation is still present in the active-turn map, permission/plan resume
can clear that stale gate and continue. This targets the GUI symptom where the
user approves a shell/plan request but continuation waits behind an already
blocked turn and eventually appears stuck.

The clear is generation-checked. The runtime records the observed active-turn
generation before reading the session snapshot, and removes the active-turn
entry only if the same generation is still present afterward. This preserves the
existing stale-release invariant: an old approval-boundary clear must not remove
a newer continuation turn that acquired the gate while the snapshot was being
read.

The change is intentionally lifecycle-level, not a model-output fallback. It
does not synthesize final answers, does not infer completion from GUI text, and
does not bypass permission decisions. It only recognizes that an approval wait
state is a runtime boundary where the current turn is no longer actively
streaming model work and the approved continuation is allowed to acquire the
next active turn.

Focused and consumer gates passed:

- `cargo test -p researchcode-runtime active_turn_generation_checked_clear_does_not_remove_new_generation -- --nocapture`
- `cargo test -p researchcode-runtime active_turn_waiting_approval_boundary_is_released_for_resume -- --nocapture`
- `cargo test -p researchcode-runtime runtime_facade -- --nocapture`
- `cargo test --manifest-path desktop/src-tauri/Cargo.toml -- --nocapture`
- `node desktop/test_runtime_event_replay.mjs`
- `node desktop/test_desktop_polish_contract.mjs`
- `npm --prefix desktop run build`

## 5. User Notification Triggers

Stop and notify before proceeding if:

1. DeepSeek or a compatible provider requires fake assistant/finalizer messages
   to continue.
2. Stable manifest plus permission gating causes unacceptable model behavior in
   live canary.
3. L1 state needs a persisted schema migration that old sessions cannot replay.
4. Blocking shell interpreters prevents an explicitly desired workflow.
5. Removing string-transition authority causes the model to terminate before
   tool results are consumed.
