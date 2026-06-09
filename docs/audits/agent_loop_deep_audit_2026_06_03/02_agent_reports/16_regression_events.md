# Agent 16: Regression Events Analysis

## Conclusion

Analysis of 40 runtime sessions (268K stream deltas, 7K tool call partials, 2,941 tool calls across 40 sessions) reveals multiple P0 issues: DeepSeek API HTTP 400 blocks, model repeating same validation errors 8+ times, Rust string panic crashing sessions, and stuck sessions with unresolved permissions. Tool call identity chains are perfect (no orphans). Convergence detection identifies non-progress but votes "continue" indefinitely.

**Severity:** P0 (HTTP 400 infinite retry, UTF-8 panic, model validation error loops)

## Files Involved

- `researchcode/runtime_desktop/runtime_session_*/events/runtime_events.jsonl` — 40 session event logs
- `desktop/.gui-smoke-runs/*/report.json` — 30+ smoke test reports
- `runs/dev_fixture_bundle_*/agent_loop_events.jsonl` — fixture data
- `runs/test_native_pending_*/events.jsonl` — test events

## Key Findings

### Finding 1: model.call_blocked by http_status_400 (P0)

15 occurrences across 7 sessions, all with `gate: "http_status_400"` and `provider: "deepseek"`. Sessions stuck in infinite retry pattern: `model.call_blocked` → loop → try again → blocked again. DeepSeek API rejecting requests. Most affected sessions: `1780377373456523000` (3x), `1780476276289592000` (3x).

### Finding 2: Model repeats same validation error 8+ times (P0)

Session `1778982897464665000`: Model called `file.edit` with missing `old_string`, `new_string`, `base_hash` **8 times consecutively**. After each rejection, repeated exact same error. Session locked into `WaitingForToolApproval` with unresolved permission.

### Finding 3: Runtime panic — UTF-8 character boundary (P0)

3 occurrences across 2 sessions. Error: `assertion failed: self.is_char_boundary(new_len)`. Session `1779074696009423000` reached `Failed` state. Multi-byte UTF-8 character split bug in Rust string handling.

### Finding 4: Same-tool-error plateau with 70-iteration delay (P0)

Session `1780379726081356000` (380 tool calls, 26,793 events): `same_tool_error_plateau` at iteration 70 before loop stop. Model stuck in repeated error cycle for 70 iterations before plateau detector terminated loop.

### Finding 5: DSML leak fallback errors (P0)

871 `deepseek.dsml.leak` events across sessions. `agent.visible_finalizer.failed` with reasons `http_status_400` and `empty_visible_response`. Fallback replaces model message with runtime-generated text.

### Finding 6: Unknown tool hallucination (P0)

Model called `create_file`, `memory_get_all` (not real tools), and `file_list_directory` instead of `file.list_directory`. Produced `tool.error.model_readable` with `UNKNOWN_TOOL` reason.

### Finding 7: Python backend telemetry black hole (P1)

10 missing event types from Python backend sessions: `agent.executor.role_call`, `agent.compactor.role_call`, `deepseek.cache.zone_a.*`, `deepseek.tool_call.partial`, `deepseek.role_split.flash_savings`, `context.compaction.*`.

### Finding 8: Model continuation prompt_hash unknown (P1)

12 of 13 continuation calls in session `1778982897464665000` have `prompt_hash: "unknown"`. Continuation reuses context but fails to record prompt hash.

### Finding 9: Duplicate tool_call_id in ledger (P1)

Session `1779076559811220000`: `duplicate tool_call_id in native turn ledger: native_loop_v2_stream_ledger_20_0`. Session reached `Failed`.

### Finding 10: Stuck sessions — unresolved permissions (P1)

Two sessions permanently stuck: one in `WaitingForToolApproval` (never decided), one in `WaitingForUser`. Sessions cannot progress without external intervention.

### Finding 11: Tool call identity chain is PERFECT (P0 ✓)

Every session: `tool.call_requested` count == `tool.call_completed` count == `tool.result_recorded` count. No orphan tool calls or missing results.

### Finding 12: Convergence loop not self-terminating (P2)

2,177 `turn.convergence.decision` events across sessions. Detector identifies non-progress patterns but votes "continue" indefinitely. Plateau detector is the only backstop.

### Finding 13: Qwen sidecar errors silent (P2)

Session `1778840437599190000`: 3 rounds, each with `sidecar_failed: exit status: 1`. Session reaches `Completed` but UI may show stale/empty responses.

## Session Completion Distribution

| State | Count | Percentage |
|---|---|---|
| Completed | 17 | 42.5% |
| Executing (truncated) | 14 | 35.0% |
| Failed | 5 | 12.5% |
| DiagnosingFailure | 2 | 5.0% |
| WaitingForToolApproval | 1 | 2.5% |
| WaitingForUser | 1 | 2.5% |

Note: No `session.completed`, `session.failed`, or `session.cancelled` event types exist. State tracking is via `session.state_changed`.

## doc39 Conflict

- **Yes** (§2.4): Model does not adapt to tool validation errors — should stop after consistent failures
- **Yes** (§7.1): No upper bound on retry iterations before plateau detection (70 iterations observed)
- **No** for event identity chain integrity

## Suggested Fix

1. Add exponential backoff or fail-fast for repeated HTTP 400 from DeepSeek API
2. Add max-consecutive-identical-error limit (stop after 3 identical validation errors)
3. Fix UTF-8 character boundary assertion in string handling
4. Add telemetry events to Python backend (cache, compaction, tool_call.partial)
5. Reduce plateau detection threshold from ~70 to ~10 iterations for same-error patterns
6. Add auto-timeout for unresolved permission requests in WaitingForToolApproval state
7. Fix prompt_hash recording on continuation calls

## Handoff Needed

- DeepSeek API integration team: investigate HTTP 400 rate root cause
- String handling team: fix `is_char_boundary` panic
- Python backend team: add missing telemetry events
