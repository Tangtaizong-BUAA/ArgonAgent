# ClaudeCode Runtime Robustness Full Repair Plan

Date: 2026-05-18

## Goal

Make the DeepSeek/Qwen local agent runtime materially closer to Claude Code's
runtime discipline for long coding tasks:

- continue from permission, plan approval, and compaction boundaries without
  restarting discovery;
- stop repeated plan/file rereads and non-productive tool loops;
- return model-readable structured errors for empty/invalid file ranges;
- preserve tool identity and evidence across resumed turns;
- reduce GUI event pressure during long streamed sessions;
- keep DeepSeek/Qwen native behavior separate from compatible providers.

This plan is an implementation contract. Once accepted, the long task should be
executed continuously until every unblocked item below is implemented and
verified, or a stop condition is reached.

## Source Boundary

Use the local ClaudeCode/OpenCode materials in this workspace as behavioral and
architectural references:

- `claude-code-main/`
- `Open-ClaudeCode-main/`
- `opencode-dev (1)/`
- `docs/engineering/architecture_upgrade_from_claude_code.md`
- `docs/implementation/architecture_upgrade_from_claude_code_todos.md`
- `docs/implementation/runtime_incident_repair_long_task_plan.md`

Do not blindly copy external implementation code into this runtime. The target
is Claude-Code-grade behavior and robustness:

- stable resume;
- bounded blocking boundaries;
- stable tool identity;
- durable/replayable events;
- model-readable recovery;
- compact evidence continuation;
- responsive GUI event consumption.

## Incident Evidence

Primary incident log:

`.researchcode/runtime_desktop/runtime_session_1779064870095449000/events/runtime_events.jsonl`

Observed failure pattern:

- one user instruction: "请继续按照plan进行编码";
- three native turns created;
- permission resume and plan approval each caused fresh discovery behavior;
- about 52 `file.read` calls and about 29 plan-file reads;
- repeated empty file ranges such as `lines=194..193`, `439..438`, `618..617`;
- duplicate observation suppression fired only twice;
- `max_tool_calls=0` allowed an effectively unbounded read loop;
- 28 DSML/fallback markup recoveries;
- 14542 `model.stream_delta` events created heavy GUI pressure;
- no context compaction was triggered, so the root cause was not context loss.

## TaskContract

### Goal

Complete every unblocked robustness repair needed to prevent the incident class:
repeated plan rereads, repeated old-content reads, resume-state loss, empty
range loops, runaway tool loops, DSML/tool-call degradation, and GUI streaming
pressure.

### Scope

Allowed implementation paths:

- `crates/runtime/src/**`
- `crates/runtime/tests/**`
- `desktop/src/**`
- `desktop/src-tauri/**`
- `desktop/*.mjs`
- `desktop/package.json`
- `scripts/**`
- `docs/implementation/**`

Allowed reference paths:

- `claude-code-main/**`
- `Open-ClaudeCode-main/**`
- `opencode-dev (1)/**`
- existing project docs under `docs/**`

Denied paths:

- secrets, private keys, `.env`, SSH keys, tokens;
- unrelated user files outside the repository;
- root `AGENTS.md` unless explicitly authorized;
- git history rewrites;
- destructive cleanup;
- dependency installs without explicit approval;
- schema/kernel/security redesign outside the narrow repair scope.

### Tool Policy

Allowed tools:

- read/search/list files;
- apply patches;
- non-destructive local tests;
- formatting/check commands;
- GUI fixture scripts that do not require live provider credentials.

Denied tools:

- destructive shell commands;
- network upload;
- package installation without approval;
- force push/history rewrite;
- reading secrets.

### Long Task Rule

When implementation begins, do not stop after a representative subset. Execute
the phases below in dependency order, continue into the next unblocked phase
after focused tests pass, and send the final report only when all unblocked
phases are complete or a stop condition is reached.

### Stop Conditions

Stop and report only if:

- a required change touches a denied path or root architecture contract;
- a dependency install or network call becomes required;
- a test exposes a blocker that requires changing scope;
- a security/permission contradiction appears;
- live provider credentials are required for further verification;
- the user explicitly pauses or redirects the task.

## Repair Phases

### Phase 1: File Read Range Correctness

Problem:

`file.read` can return successful empty ranges such as `lines=194..193`, which
teaches the model to retry with more offsets.

Implementation:

- make `slice_lines` return structured metadata including total lines and empty
  range status;
- when requested offset is past EOF or normalized range is empty, return
  `ok:false`;
- use `error_code:"READ_RANGE_EMPTY_OR_EOF"`;
- include `line_count`, `requested_offset`, `requested_limit`, `valid_range`,
  and `next_action_hint`;
- ensure empty/EOF reads are not marked `truncated=true`;
- keep valid partial reads unchanged.

Tests:

- focused Rust test for offset past EOF;
- focused Rust test for limit zero or normalized empty range;
- regression assertion that preview never says `lines=N..N-1`.

Acceptance:

- model receives a clear non-success result and guidance to stop paging that
  file range.

### Phase 2: Overlap-Aware Observation Cache

Problem:

Duplicate suppression only catches exact argument matches, not overlapping
reads such as `1..193`, `51..110`, `101..160`, `401..617`, `401..1052`.

Implementation:

- extend `ObservationCache` to track normalized file-read intervals per path;
- treat fully covered ranges as duplicate observations;
- allow non-overlapping continuation ranges;
- allow larger reads only for the not-yet-covered suffix/prefix or return a
  model-readable duplicate result;
- emit `tool.duplicate_observation_suppressed` with
  `reason:"covered_file_range"`;
- update duplicate result guidance from "exact observation" to "already covered
  observation".

Tests:

- exact read duplicate still suppressed;
- overlapping covered read suppressed;
- adjacent continuation range allowed;
- broader range with new coverage allowed or narrowed according to final
  implementation policy.

Acceptance:

- repeated plan/file reads are blocked before another expensive tool execution.

### Phase 3: Evidence Ledger Across Turns

Problem:

Permission resume and plan approval create new native turns without a hard
summary of already inspected evidence.

Implementation:

- add an `EvidenceLedger` or equivalent session-memory summary inside
  `RuntimeFacade`;
- ingest native loop `tool.result_recorded` events into runtime file state and
  evidence memory during live event merge;
- track read files, read ranges, content hashes, directory/list observations,
  search observations, completed writes, permission decisions, and active plan
  state;
- inject an "Already Executed Evidence" block into every new live native turn;
- after permission resume or plan approval, inject explicit constraints:
  "do not reread these plan/file ranges; continue implementation";
- record a replayable event such as `runtime.evidence_ledger.injected`.

Tests:

- facade event ingestion updates file state from native loop events;
- second turn prompt includes prior file evidence;
- permission resume prompt includes "already read" constraints;
- plan approval continuation does not start with a blank discovery context.

Acceptance:

- new turns no longer behave like a fresh task unless the user actually starts
  one.

### Phase 4: Resume And Plan Approval Continuation Semantics

Problem:

`file.write` approval and `plan.enter` approval currently behave like turn
boundaries that invite rediscovery.

Implementation:

- preserve pending native tool provider id and ledger id exactly once;
- after permission approval, continue with the executed tool result plus
  evidence ledger, not a fresh user-prompt-only request;
- after `plan.enter` approval, convert the approved plan into active task
  contract context instead of opening another discovery turn;
- add explicit model instruction that plan approval is not permission to reread
  the plan;
- reject stale approval cards and stale pending decisions with structured
  events.

Tests:

- permission resume executes the pending tool once;
- resume continuation includes prior evidence and tool result;
- `plan.enter` emits plan lifecycle events but does not discard evidence;
- stale approval returns clear error and clears GUI state.

Acceptance:

- approval boundaries become continuation points, not task resets.

### Phase 5: Tool Loop Budget And Exploration Brake

Problem:

`max_tool_calls=0` can mean effectively unlimited tools. Long sessions can keep
reading/searching without implementation progress.

Implementation:

- normalize `max_tool_calls=0` to a bounded default in live GUI/runtime paths;
- add separate exploration budgets:
  - plan reads per turn;
  - repeated reads per file;
  - read/list/search-only iterations;
  - consecutive non-progress iterations;
- when budget is hit, emit `agent.loop_budget_reached` with
  `reason:"exploration_budget_exhausted"`;
- return model-readable guidance to implement or summarize from collected
  evidence;
- preserve a higher cap for explicit review/research tasks.

Tests:

- GUI live request with `0` gets a bounded effective cap;
- repeated plan reads trip exploration budget;
- implementation/write path is not blocked by read exploration cap;
- existing max-tool-call finalizer tests still pass.

Acceptance:

- the model cannot spend dozens of tool calls rereading plans before acting.

### Phase 6: DeepSeek/Qwen Tool Protocol Hardening

Problem:

DSML/fallback markup leaks were recovered but frequent, adding event noise and
tool-history confusion.

Implementation:

- strengthen native prompt/tool manifest instructions for DeepSeek and Qwen;
- keep DSML fallback for recovery but emit severity telemetry when repeated;
- after repeated DSML leak in one turn, inject a protocol correction message;
- preserve DeepSeek/Qwen native parser separation from compatible providers;
- ensure wrong-tool execution still blocks native eval promotion.

Tests:

- DSML fallback still recovers one malformed tool call;
- repeated DSML leak emits escalation telemetry;
- compatible provider paths do not inherit native-only policy.

Acceptance:

- malformed tool markup becomes a recoverable exception path, not the common
  tool-call path.

### Phase 7: GUI Stream Pressure Reduction

Problem:

The incident produced 14542 `model.stream_delta` events, making the GUI feel
stuck during long runs.

Implementation:

- coalesce reasoning stream deltas on the backend or facade ingestion boundary;
- batch high-frequency stream events before pushing to GUI;
- keep final visible text and tool lifecycle events lossless;
- ensure event log remains replayable;
- keep frontend virtualization/windowing intact;
- add event-rate telemetry such as `runtime.stream.coalesced`.

Tests:

- fixture with dense stream deltas renders fewer GUI events;
- final transcript content remains correct;
- tool events are not dropped;
- `desktop/gui_three_round_smoke.mjs --incident-verify` still passes.

Acceptance:

- long-running sessions remain responsive while preserving replayability.

### Phase 8: Incident Fixture And Regression Harness

Problem:

The current incident can be diagnosed manually but should become an automated
regression.

Implementation:

- add a script or test fixture that analyzes the incident JSONL;
- assert excessive repeated plan reads are detected in the old log;
- add a synthetic native loop fixture that tries repeated overlapping plan
  reads and verify suppression;
- add GUI incident command if needed.

Tests/checks:

- `cargo test -p researchcode-runtime native_agent_loop_v2_ -- --nocapture`
- `cargo test -p researchcode-runtime agent_kernel:: -- --nocapture`
- focused `tool_execution` and `observation_cache` tests;
- `cargo fmt --check`;
- `node --check desktop/gui_three_round_smoke.mjs`;
- `npm run gui:incident-fixture`;
- optional: `npm run gui:incident-live` only when live GUI/provider approval is
  explicitly allowed.

Acceptance:

- the original failure class is reproducible as a failing detector and covered
  by focused passing regression tests after the repair.

## Execution Order

1. Implement Phase 1 because empty EOF ranges are the smallest root cause with
   highest leverage.
2. Implement Phase 2 so repeated reads are stopped before runtime/model prompt
   changes.
3. Implement Phase 3 to carry evidence into new turns.
4. Implement Phase 4 to make approval boundaries true continuations.
5. Implement Phase 5 to prevent future non-productive tool loops.
6. Implement Phase 6 to reduce malformed native tool-call churn.
7. Implement Phase 7 to reduce GUI streaming pressure.
8. Implement Phase 8 to lock the incident class into regression coverage.

## Required Final Report

The final implementation report must include:

- changed files;
- concise summary;
- tests/checks run;
- risks;
- unresolved questions;
- next recommended task.

If any phase is blocked, the report must include:

- exact phase;
- evidence;
- why it is a real stop condition;
- what remains safe and already completed.

## Definition Of Done

This task is not done until:

- empty/EOF file reads are structured non-success results;
- overlapping file/plan reads are suppressed or narrowed;
- evidence survives permission resume and plan approval;
- approval continuation does not restart discovery;
- tool budgets prevent long reread loops;
- DSML leak recovery has escalation telemetry;
- GUI event pressure is reduced for dense streams;
- focused Rust tests and practical GUI fixture checks pass or have documented
  stop-condition blockers.
