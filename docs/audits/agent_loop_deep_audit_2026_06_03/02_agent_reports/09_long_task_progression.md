I have all the information needed. Let me compile the comprehensive audit report.

---

## Audit Report: Long Task Progression System

### Conclusion

The current system lacks any explicit task-phase tracking (EvidenceCollected, WriteExpected, WriteCompleted, ShellVerifyExpected, ShellVerified, ReadyToSummarize). Progression from reading to writing to shell-verification is entirely model-driven, with the runtime providing only weak hints via tool-result previews and plateau-detection that eventually kills the loop. There is no runtime component that tracks "what has been collected" against "what is needed" or promotes write/verify intent after sufficient evidence is gathered. A stress test producing 50 rounds of reads without reaching a shell is the expected behavior of the current architecture.

### Severity: **P2**

This is a design-level gap, not a bug. The system works for short tasks (1-5 iterations) because the model naturally transitions from read to write. For long-horizon tasks (50+ planned rounds), the absence of phase-aware progression causes wasted iterations, budget exhaustion, and user-facing stalls.

### Files Involved

- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/agent_kernel/turn_state.rs` -- `ToolProgressState`, `TurnState`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/agent_kernel/turn_controller.rs` -- `NativeLoopTurnController`, plateau detection, convergence handling
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/agent_kernel/evidence_ledger.rs` -- `EvidenceLedger`, `EvidenceClass`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/agent_kernel/convergence_enforcer.rs` -- `ConvergenceEnforcer`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/native_agent_loop.rs` -- Main loop (lines 556-2879), the `for iteration` loop
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/native_agent_loop_tools.rs` -- `execute_duplicate_observation_collect` (the "hint" in the preview), `tool_calls_are_cached_observations`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/native_agent_loop_execution.rs` -- Tool execution dispatch
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/agent_kernel/kernel.rs` -- `AgentKernel` service graph
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/native_turn_controller.rs` -- `NativeTurnController`
- `/Users/gongyuxuan/Documents/deep-code/crates/runtime/src/native_agent_loop_completion.rs` -- `stop_native_loop_with_structured_failure`

### Events Involved

- `agent.loop_recovery` -- with reasons `non_progress_iteration`, `repeated_non_progress`, `repeated_tool_batch`, `duplicate_tool_observation_plateau`, `non_progress_tool_plateau`, `same_tool_error_plateau`
- `agent.loop_plateau_stopped` -- with reasons `duplicate_dominated_plateau`, `information_stagnation_plateau`, `batch_novelty_plateau`, `convergence_budget_exhausted`
- `agent.write_progress.blocked` -- Fired when a write-oriented task stops without any state change
- `agent.convergence_escalation` -- Fired when escalation to CodeEdit exposure occurs
- `turn.convergence.decision` -- Records each convergence decision with verdict and source
- `tool.duplicate_observation_suppressed` -- Fired for each duplicate observation that is suppressed
- `agent.visible_only_transition_detected` -- Fired when model produces only transitional narration without tool calls

### State Involved

1. **`ToolProgressState`** (turn_state.rs lines 78-88):
   - `new_observation_keys`, `recovery_results`, `duplicate_results`, `error_results` -- cumulative counts
   - `consecutive_no_progress_iterations` -- increments when no new observation keys
   - `consecutive_duplicate_iterations` -- increments when only duplicates
   - `repeated_error_signature` / `repeated_error_streak` -- tracks repeated errors
   - Decisions: Continue, SoftWarning (at 3 consecutive), Stop (at 6 consecutive errors)

2. **`EvidenceLedger`** (evidence_ledger.rs):
   - Per-iteration items with `EvidenceClass` (NewEvidence, Recovery, Error, Suppressed)
   - `no_new_evidence_streak()` -- reads sealed iteration history
   - `duplicate_dominated_streak()` -- checks if >=70% of items are Suppressed

3. **`ConvergenceEnforcer`** (convergence_enforcer.rs):
   - `duplicate_ratio_threshold: 0.7`, `duplicate_window: 2`, `no_new_evidence_window: 3`, `batch_window: 6`
   - `observe_iteration()` returns: `DuplicateDominance`, `InformationStagnation`, `BatchNoveltyPlateau`, `BudgetExhausted`, `Continue`

4. **`NativeLoopTurnController`** (turn_controller.rs):
   - `max_loop_guard_recoveries: 2` -- how many repeated batch synthetic recoveries before exhaustion
   - `max_escalation_attempts: 2` -- how many times to escalate to CodeEdit
   - `non_progress_recovery_count` -- resets when any ok_result arrives

5. **`TurnState`** (turn_state.rs):
   - `seen_tool_batches: Vec<String>` -- batch signatures
   - `observation_cache: ObservationCache` -- distinct key count
   - Does NOT track: which files were read, what content was collected, what is needed next

### Reproduction Clues

A long-horizon task where the model repeatedly reads files (e.g., exploring a large codebase) will:
1. The model emits `file.read` calls for iteration 0.
2. Iteration 1 onward: the model may emit the same `file.read` calls. `tool_calls_are_cached_observations` returns true.
3. The loop at line 1627 hits `ToolBatchGuardAction::Continue` (not Stop) because `repeated_cached_observation_batch` is true (line 381 in turn_controller.rs skips the repeated-batch guard).
4. Tool results are returned as `EvidenceClass::Recovery` -- the model sees results saying "duplicate, here is the cached evidence."
5. The model sees no reason to stop reading, so it continues proposing reads.
6. `ConvergenceEnforcer.DuplicateDominance` fires only after 2 iterations with >=70% suppression.
7. `InformationStagnation` fires only after 3 consecutive iterations with zero new evidence.
8. Even then, the action is `SoftWarning` (continues loop) or `EscalateToCodeEdit` (just adds write tools but does not force writes).
9. The loop stops only via budget exhaustion (max_iterations) or if the model voluntarily switches to writing.

### Evidence Fragments

From `execute_duplicate_observation_collect` in `native_agent_loop_tools.rs` (line 993-1067), the tool result preview is:
```
"File unchanged since the earlier {tool_id} call in this conversation. The previous tool_result is still current; refer to it instead of re-reading. If you have all evidence needed, produce the final answer/report now. For code-edit tasks only, proceed to write/edit/patch instead of further reads."
```

This is a model-readable string -- the model may or may not follow it. There is no runtime enforcement.

From `ToolProgressState::record_iteration` in `turn_state.rs` (line 132-177):
- `new_observation_keys > 0` => Continue (resets all streaks)
- `duplicate_results > 0` for 3 consecutive iterations => `SoftWarning` only, never Stop
- `no_progress` (no new, no recovery, no errors, no duplicates) for 3 iterations => `SoftWarning`

From `ConvergenceEnforcer::observe_iteration` in `convergence_enforcer.rs` (line 42-88):
```rust
if duplicate_ratio >= self.duplicate_ratio_threshold  // 0.7
    && ledger.duplicate_dominated_streak() >= self.duplicate_window  // 2
{
    return ConvergenceVerdict::DuplicateDominance { ... };
}
if distinct_keys_growth == 0
    && ledger.no_new_evidence_streak() >= self.no_new_evidence_window  // 3
{
    return ConvergenceVerdict::InformationStagnation { ... };
}
```
Both of these map to `SoftWarning` (not Stop) unless `can_escalate_to_code_edit` is true AND `escalation_attempts < max_escalation_attempts` (2), in which case they become `EscalateToCodeEdit`.

From `observe_convergence` in `turn_controller.rs` (line 1060-1156):
- `DuplicateDominance` or `InformationStagnation` with escalation available => `EscalateToCodeEdit`
- Without escalation => `SoftWarning`
- `BudgetExhausted` => `Stop`
- `BatchNoveltyPlateau` => `SoftWarning`

### Root Cause

**The system has no concept of task-level phases.** The `NativeAgentToolExposure` (ReadOnly, FastAutoWrite, CodeEdit) is determined once at turn start via `TurnRouter::classify()`, and can only escalate upwards (ReadOnly -> CodeEdit) via plateau detection. But:

1. Escalation only adds tools to the manifest; it does not force the model to stop reading.
2. There is no "evidence sufficiency" check -- no comparison of what has been collected against what the task requires.
3. There is no write-verification phase -- after writing, there is no mechanism to say "now verify with shell."
4. The loop termination is entirely negative (budget exhausted, plateau detected) rather than positive (phase goals achieved).

The common path: model reads -> loop feeds back duplicate evidence -> model reads more. The plateau detectors take 3+ iterations to trigger soft warnings, and the warnings themselves do not stop the loop. Without an explicit progression model, the model can stay in the "read" phase indefinitely.

### Hidden Risks

1. **Duplicate evidence inflation**: Each suppressed duplicate creates a `Recovery`-class evidence entry. The model may interpret this as "new information" even though it's a dedup hint, leading the model to believe it is making progress when it is not.

2. **SoftWarning-but-continue is invisible**: `ToolIterationControlAction::SoftWarning` fires events but does not block the iteration. The model never sees the SoftWarning directly; it only sees the tool results. The warning events are for telemetry, not model guidance.

3. **Escalation without content change**: `EscalateToCodeEdit` adds `file.write`/`file.edit` tools but does not provide a prompt stating "you have been escalated because you are stuck." The model continues in the same context, unaware of the escalation.

4. **Structured failure kills the turn, not promotes progression**: When plateaus trigger a stop, the turn ends with `stop_native_loop_with_structured_failure` and transitions to `AgentState::Failed`. There is no "retry with different phase" mechanism.

5. **`last_tool_batch.clear()` at line 1705**: After each tool batch is processed, the legacy batch is cleared. The `evidence_ledger.clear()` call at line 1706 also clears the current iteration. This means the evidence sent back to the model for continuation is always just the most recent batch -- early-iteration evidence is only in the `history_digest` (text summary), not in structured tool results. The model may "forget" what it discovered.

6. **`async task.dispatch` as subagent**: When a task dispatches a subagent, the subagent runs with its own `NativeAgentToolExposure` (typically `ReadOnly` for explorers). The subagent is not phase-aware either.

### doc39 conflict: **No**

None of the proposed phases (EvidenceCollected, WriteExpected, etc.) exist in the current system, so there is no conflict with existing designs. Implementing them would be additive.

### Suggested Fix

**Add a `TaskProgressLedger` struct** alongside the existing `EvidenceLedger` that tracks:

1. **Phase enum**: `CollectingEvidence -> WritingCode -> ShellVerifying -> Summarizing`
2. **Phase transitions**: A set of guards that decide when to transition:
   - `CollectingEvidence -> WritingCode`: When the model has submitted write-type tools (file.write, file.edit, patch.apply, shell.post) OR when observation_cache distinct_key_count has plateaued for N iterations AND the task hint involves code generation.
   - `WritingCode -> ShellVerifying`: When a write tool succeeds AND the task hint implies shell verification (e.g., "run tests", "verify with shell").
   - `ShellVerifying -> Summarizing`: When a shell command succeeds AND it is the verification step.

3. **Phase-aware loop iteration**:
   - Before each iteration, inject a system-level hint indicating the current phase (e.g., "You are in the Writing Code phase. Focus on file writes, edits, and patches.").
   - When phase transition is detected, inject a phase-transition message into the tool batch.

4. **Write-intent promotion**: When the system detects N iterations of pure reads with sufficient coverage (e.g., `ObservationCache` shows many distinct files read), promote the tool exposure and inject a guidance message: "You have read enough files. Now switch to write tools to implement the solution."

5. **Shell-verify intent**: After a file.write/file.edit succeeds and the manifest allows shell.command, inject a continuation hint: "File written. Now run the verification command."

6. **Phase tracking in `TurnState`**: Add `phase: TaskPhase` field and integrate it into `AgentKernel` so it is available during loop orchestration.

### Not Suggested

1. Do not hardcode phase transitions based purely on tool call counts -- the model should still drive sequencing. The runtime should guide, not gate.
2. Do not remove the plateau detection -- it serves as a safety net for infinite loops. The phase system should layer on top.
3. Do not add mandatory phase progression (cannot skip from CollectingEvidence directly to ShellVerifying without WritingCode) -- the model may handle simple tasks in one step.
4. Do not modify the `EvidenceLedger` to hold phase data -- keep it as a tool-result accumulator only.

### Handoff Needed

This audit identified a design-level gap. Before implementation, the following need resolution:

1. **Phase definitions**: The exact phase enum variants and their transition conditions need team agreement. The current proposal (CollectingEvidence -> WritingCode -> ShellVerifying -> Summarizing) is a starting point.
2. **Prompt integration**: How phase hints are injected into the model context -- as system messages, user messages, or tool-result hints. Each has different behavioral characteristics with different model families (DeepSeek vs Qwen).
3. **Backward compatibility**: The phase system should be optional (default: no phase tracking, current behavior) until explicitly enabled, to avoid regressions in existing tasks.
4. **Phase timeout**: What happens if a phase takes too long? Should there be a timeout per phase or a total budget limit?
5. **Testing strategy**: The existing plateau tests (turn_state.rs tests, turn_controller.rs tests) do not cover phase transitions. New tests are needed that simulate long-running tasks with explicit phase tracking.