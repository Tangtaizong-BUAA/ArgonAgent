# Post-Audit Corrected Architecture Remediation Plan

Date: 2026-06-03
Status: active planning correction
Scope: agent loop, provider stream, permission resume, TCML, context, GUI lifecycle, and deterministic/live stress gates.

Current execution overlay:

- `10_current_implementation_replan_2026_06_04.md`

The overlay preserves this document as the post-audit contract, but reclassifies
items that have already landed in the current worktree and defines the next
implementation order from the live code state.

This document corrects the execution order and severity model after reading the full audit package:

- `00_scope.md`
- `02_agent_reports/01-23`
- `03_issue_matrix.md`
- `04_doc39_conflict_matrix.md`
- `04_architecture_review.md`
- `05_test_system_review.md`
- `06_remediation_grading.md`
- `05_red_team/01-03`
- `08_final_audit_package.md`
- doc39 and doc40 planning documents

It does not replace doc39. It is the corrected implementation contract for turning the audit into engineering work without reintroducing finalizer-style patches, workflow-specific tool hiding, or hard loop-cap control.

## 1. Corrected Diagnosis

The current failure class is not "DeepSeek is bad at long tasks" and not "one missing if statement". It is a pipeline architecture problem:

1. AgentKernel is still mostly a pass-through wrapper while the monolithic native loop owns orchestration.
2. Stream events can affect loop/tool state before the provider response is known to be successful.
3. Permission, plan approval, cancel, and active-turn state do not all carry a turn generation.
4. Convergence detects no-progress but often emits warnings/escalations instead of a single structured stop.
5. Tool manifest/exposure still carries workflow steering semantics that should belong to permission and prompt guidance.
6. TCML is not the sole tool mediation path because concurrent read-only execution bypasses mediation.
7. GUI turn completion is still too easy to infer from content/tool-card settling instead of lifecycle events.
8. Context compression exists, but lacks a stable L1 state object and reversible pointer model.

The first repair wave must therefore protect event/state correctness before doing large AgentKernel migration. Otherwise every large refactor will be debugging through dirty events, stale approvals, and false completion states.

## 2. Severity Corrections

### Upgraded or Reclassified

| Finding | Corrected severity | Reason |
|---|---:|---|
| AgentKernel hollow pass-through | P0 architecture | It invalidates the "kernel owns orchestration" claim. Treat as a staged migration target, not a quick patch. |
| Dirty structural stream events before 2xx | P0 | Failed Anthropic attempts can mutate tool/ledger state before OpenAI fallback succeeds. |
| Convergence warning without stop | P0 | Explains long non-error plateaus and false "continue forever" behavior. |
| `visible_text_looks_like_transition_statement` as loop control | P1 conflict | It is a string heuristic for loop control. It may remain telemetry, not authority. |
| Active turn without generation | P1/P0 depending path | Causes "stop then next question hangs" and stale task release clobbering. |
| Native shell path skipping facade hard-deny | P1 security | Security guarantees depend on entrypoint. Must unify. |
| Shell interpreters not denied | P0 security | `sh`, `bash`, `zsh`, `dash` should never be directly invokable through `shell.command`. |
| Prompt/route to tool exposure chain | P1 doc39 conflict | Code still maps `TurnRoute` to `NativeAgentToolExposure`; this is workflow-state manifest steering. |

### Downgraded or Clarified

| Finding | Corrected severity | Reason |
|---|---:|---|
| Separate Flash compactor missing | P2 partial | Separate Flash is a cost/latency role. The correctness gap is L1 state, reversibility, and pointer replay. |
| Classifier Deny user-approvable | P1 UX/security consistency | Execution backstop blocks many denied commands, but UI and permission model are misleading. |
| MCP and real subagent absence | Later capability track | Important, but not first-wave blocker for current shell/approval/loop correctness failures. |
| Full ripgrep parity | Later capability track | Search quality matters, but should not delay event/permission/loop correctness fixes. |

## 3. Non-Negotiable Invariants

1. No `agent.final_answer` or `visible_finalizer_answer` as an interactive-loop primitive.
2. A model turn ends naturally when the assistant produces no tool calls and has assistant text, or structurally with blocked/failed/interrupted/awaiting events.
3. Tool results always go back to the model unless the turn is awaiting approval, interrupted, or structurally failed.
4. Tool manifest is stable and complete for the session/provider capability; PermissionPolicy gates execution.
5. TCML is the only mediation path for model tool calls, including concurrent/read-only batches.
6. Provider stream callbacks must not mutate tool execution state until the HTTP attempt is accepted.
7. Permission and active-turn resume must be tied to a turn generation.
8. GUI composer/status derives from lifecycle events, not transcript content or tool-card completion.
9. DeepSeek/Qwen native profile behavior stays first-class and separated from compatible provider behavior.

## 4. Task Contract

### Goal

Repair the runtime/GUI agent loop so long DeepSeek/Qwen native turns can:

- plan, wait for approval, and continue;
- request shell/write permissions, resume exactly once, and continue;
- stream assistant text around tools without dropping narration;
- stop no-progress loops structurally without fabricating final answers;
- survive cancel and immediate follow-up;
- preserve event-log replay and context compaction invariants.

### Allowed Paths

- `crates/runtime/src/**`
- `crates/kernel/src/**`
- `desktop/src/**`
- `desktop/src-tauri/src/**`
- `desktop/gui_*.mjs`
- `desktop/test_*.mjs`
- `scripts/check_all.py` and focused validation scripts
- `docs/**`
- deterministic fixtures under repo-controlled fixture directories

### Denied Without New Approval

- root `AGENTS.md`
- git history rewrite
- destructive cleanup
- secrets/private keys/SSH files
- network/package install
- changing DeepSeek/Qwen native strategy without eval fixture and rollback plan

### Stop Conditions

Stop and report if:

- a kernel/event/security schema needs a breaking migration without compatibility;
- a provider protocol requires fake assistant/finalizer messages to remain valid;
- a deterministic gate exposes a contradiction between doc39 and current implementation;
- live-provider testing requires network permission that has not been granted in the current environment.

## 5. Implementation Phases

### Phase 0: Freeze The Safety Rail

Purpose: prevent the old failure modes from returning while code is moved.

Actions:

1. Add architecture-guard tests that fail on production exposure of:
   - `agent.final_answer`
   - `visible_finalizer_answer`
   - `disable_tools_and_request_final_answer`
   - `model_continuation_skipped`
   - tool-disabling completion paths
2. Add an audit regression fixture that asserts `visible_text_looks_like_transition_statement` cannot be the sole reason a write/tool task completes.
3. Record the current dirty-worktree baseline before implementation slices.

Acceptance:

- Existing no-finalizer tests still pass.
- New guard test proves string transition detection is not a terminal authority.

### Phase 1: Event, Permission, Cancel Correctness

Purpose: fix the user-visible hangs and false completions first.

Actions:

1. Buffer all structural stream events until provider 2xx status is confirmed.
2. On protocol fallback, discard or tombstone all events from the failed attempt before retry.
3. Add turn generation to desktop/local-api `active_turns`; stale task release must not clear a newer turn.
4. Add `turn_id`/generation to `RuntimePendingNativeDecision`; reject stale approval decisions.
5. Clear pending native decisions on cancel and terminal failure.
6. Add exactly-once guard for permission resume.
7. Unify shell hard-deny coverage across facade and native loop paths.
8. Block shell interpreters and dangerous programs at the token/program layer.
9. Normalize classifier `Deny` so UI does not present it as ordinary approvable work.

Required tests:

- dirty Anthropic 400 stream attempt produces no tool execution state.
- fallback retry can succeed without duplicate tool ids.
- shell permission approve executes once and continues to the model.
- double approval does not execute twice.
- cancel while waiting for approval clears stale pending decision.
- stop then immediate next message cannot be clobbered by old task release.
- `sh -c`, `bash -c`, `zsh -c`, `sudo`, `dd`, `mkfs`, `fdisk`, `systemctl` are denied before user approval.

### Phase 2: Loop State And Convergence Authority

Purpose: convert the loop from scattered local state into traceable turn state.

Actions:

1. Introduce `NativeLoopState` with:
   - current iteration;
   - assistant/tool batch staging;
   - provider attempt state;
   - transition reason;
   - continuation strategy;
   - no-progress counters;
   - phase hints;
   - terminal reason.
2. Replace ad-hoc `continue` sites with named transitions.
3. Make convergence decisions route through one stop authority.
4. Turn `DuplicateDominance` and `InformationStagnation` into structured stop after a bounded warning window.
5. Remove mid-loop `EscalateToCodeEdit` manifest mutation. If escalation remains useful, make it a new-turn prompt/permission hint, not an in-turn tool-surface change.
6. Stage tool results locally and commit them atomically to the continuation view.

Required tests:

- duplicate read plateau stops as `Blocked`/`Failed`, not `Completed`.
- interleaved success on tool A cannot reset repeated error for tool B.
- no-progress without identical errors still stops structurally.
- transition reason appears in events for every loop continuation and terminal state.

### Phase 3: Manifest Stability And TCML Sole Path

Purpose: implement doc39's "manifest does not steer, permission steers".

Actions:

1. Remove `TurnRoute -> NativeAgentToolExposure -> ToolManifestExposure` as normal tool-surface policy.
2. Keep all enabled/core tools in the model-visible manifest unless disabled by provider capability, user policy, feature gate, or explicit deny rule.
3. Move read-only/plan/manual review semantics to PermissionPolicy and prompt guidance.
4. Unify `tui_fastauto` schema generation with the main manifest builder.
5. Enforce `model_compatibility`.
6. Route concurrent read-only tools through TCML before dispatch.
7. Use canonical tool IDs and canonical argument JSON after mediation.

Required tests:

- `ReadOnly` route still exposes write/shell schemas, but execution policy asks/denies as configured.
- plan mode approval is an execution permission state, not a manifest state.
- aliases and relational defaults work in concurrent batches.
- every model tool call emits TCML mediation events before execution.

### Phase 4: Provider Projection And Stream Assembly

Purpose: remove malformed provider requests and unobservable incomplete tool calls.

Actions:

1. Replace manual JSON request construction with typed serialization for native and compatible provider paths.
2. Fix compatible Anthropic content blocks for tool use and tool result.
3. Remove non-standard `reasoning_content` fields from Anthropic-compatible request bodies; keep protocol-native thinking blocks where supported.
4. Make conversation history projection typed; remove text-marker coupling for OpenAI history.
5. Flush incomplete streaming tool calls on stream completion as model-readable parse errors with telemetry.
6. Record and expose `finish_reason`.
7. Parse tool-result error status as structured data, not substring search.

Required tests:

- compatible Anthropic tool-call history serializes into valid structured blocks.
- DeepSeek OpenAI `tool_calls.delta` split across chunks assembles correctly.
- incomplete tool arguments do not disappear silently.
- fallback from Anthropic to OpenAI does not reuse dirty processor state.

### Phase 5: Context Spine And Reversible Compaction

Purpose: land the user's L0-L4 memory design in a way compatible with doc39.

Actions:

1. Add a compact L1 state object:
   - overall goal;
   - current subgoal;
   - confirmed facts with refs;
   - decisions;
   - open questions;
   - touched resources;
   - next expected action.
2. Keep L0/L1 pinned in every model request.
3. Keep only the recent action-observation window raw in L2.
4. Fold older observations into L3 summary records with `ref://` pointers.
5. Store raw event/tool output in L4 event archive and page back by pointer.
6. Preserve latest required DeepSeek raw reasoning for adjacent tool-result replay.
7. Improve token estimation; API token counting is preferred but optional behind capability/proxy constraints.
8. Treat separate Flash compactor as a later optimization unless product eval proves in-process summaries are insufficient.

Required tests:

- context above threshold compacts without losing L1 goal/state.
- old observation can be referenced by pointer and paged back.
- DeepSeek reasoning replay survives compaction for the next required turn.
- compacted state does not inject fabricated final answers.

### Phase 6: GUI Lifecycle And Transcript Rendering

Purpose: make the GUI reflect the event truth without swallowing text or freezing composer state.

Actions:

1. Use block-ordered transcript events for assistant text, reasoning preview, tool call, and tool result.
2. Render Markdown incrementally enough for streaming without reparsing the whole transcript on every frame.
3. Reset stream lifecycle refs by generation, not boolean timers.
4. Cap transcript memory and dedupe state with a stable LRU/window strategy.
5. Show tool failures and recoverable errors without marking the turn complete.
6. Composer state follows lifecycle events:
   - `agent.turn.completed`
   - `agent.turn.failed`
   - `agent.turn.blocked`
   - `agent.turn.interrupted`
   - `agent.turn.awaiting_permission`
   - `agent.turn.awaiting_plan`

Required tests:

- pre-tool text remains visible.
- inter-tool narration remains visible.
- post-tool answer remains visible.
- approval modal does not freeze composer after approve.
- stopping a turn releases composer only after runtime interruption is acknowledged.
- 1000+ event replay stays responsive.

### Phase 7: Deterministic Harness Before Live DeepSeek

Purpose: separate harness/runtime bugs from model behavior.

Required deterministic fixtures:

1. shell approval -> tool executes -> model continuation.
2. plan approval -> synthetic artifact -> model continuation.
3. TCML full chain: alias -> schema -> repair -> permission -> dispatch.
4. Anthropic 400 fallback with structural stream events buffered.
5. incomplete tool call at stream end.
6. cancel with pending native decision then new user turn.
7. active-turn generation stale release.
8. large tool result and 1000+ event GUI replay.
9. subagent lifecycle event replay, even before real subagent execution is implemented.

Live DeepSeek canary should run only after the deterministic suite passes. Its purpose is drift detection, not primary correctness proof.

### Phase 8: AgentKernel Ownership Migration

Purpose: remove the architectural root cause after safety gates exist.

Actions:

1. Move orchestration ownership from `native_agent_loop.rs` into `AgentKernel::run_turn`.
2. Keep `NativeProfile` responsible for DeepSeek/Qwen stream/request/parser behavior.
3. Unify initial and continuation paths into one loop iteration pipeline.
4. Replace inline JSON event construction with typed event builders.
5. Remove or feature-gate dead/legacy resume paths that duplicate the main loop.
6. Merge overlapping turn controller/ledger responsibilities behind one kernel-facing authority.

Acceptance:

- `AgentKernel` services are used by `run_turn`, not recreated and bypassed.
- runtime facade delegates loop policy to kernel only.
- all model tool calls pass through TCML and PermissionGate.
- replay/event invariant suite passes.

## 6. Defer To Capability Track

The audit correctly identifies these as important, but they are not first-wave blockers for the current user-visible failures:

- real MCP dynamic tool registration;
- full OpenClaudeCode-grade subagent spawning;
- ripgrep feature parity;
- streaming tool execution overlap;
- provider abstraction trait cleanup;
- model fallback between providers;
- stop hooks/post-sampling hooks;
- full async transport migration.

They should be tracked after Phase 1-7 correctness gates are green.

## 7. Required Validation Gates

Focused Rust gates:

```text
cargo test -p researchcode-runtime <focused_test_name> --lib
cargo test -p researchcode-kernel <focused_test_name> --lib
```

Desktop deterministic gates:

```text
npm --prefix desktop run build
node desktop/test_runtime_event_replay.mjs
node desktop/test_desktop_polish_contract.mjs
node desktop/gui_permission_longtask_smoke.mjs
node desktop/gui_full_stack_regression.mjs
```

Broad gate:

```text
python3 scripts/check_all.py
```

Live gated canary:

```text
npm --prefix desktop run gui:argon-longtask-live
```

Live canary must report:

- plan approval requested and decided;
- permission requested and decided;
- shell command executed after approval;
- model continuation after approved shell result;
- no `agent.final_answer`;
- no `visible_finalizer`;
- no `model_continuation_skipped`;
- no premature GUI `Completed`;
- no unresolved active turn after terminal event.

## 8. First Implementation Slice

The next coding slice should not start with AgentKernel migration. It should start with the smallest cross-cutting set that makes future refactors debuggable:

1. stream-event attempt buffering and fallback tombstoning;
2. active-turn generation in desktop and local API;
3. pending decision generation and cancel cleanup;
4. deterministic shell permission resume fixture;
5. deterministic plan approval resume fixture;
6. no-progress convergence stop fixture.

Only after those pass should manifest/TCML and kernel migration start.

## 9. Must Notify User

Stop and ask before choosing if:

1. DeepSeek requires fake assistant/finalizer messages for a provider protocol to accept continuation.
2. Stable manifest with permission gating causes unacceptable model behavior in real DeepSeek canary.
3. L1 state object needs a new persisted schema that cannot replay old sessions.
4. Blocking shell interpreters prevents an explicitly desired safe shell workflow.
5. Removing route-to-exposure mapping breaks a current product mode with no permission-policy replacement.

## 10. Summary

The corrected strategy is:

```text
first make events, approvals, cancellation, and provider attempts truthful;
then make loop/convergence state explicit;
then make manifest/TCML match doc39;
then repair provider projection and context spine;
then harden GUI lifecycle;
then migrate AgentKernel ownership.
```

Do not begin with finalizer fixes, tool hiding, or hard loop caps. Those were the patch pattern that created the current class of failures.
