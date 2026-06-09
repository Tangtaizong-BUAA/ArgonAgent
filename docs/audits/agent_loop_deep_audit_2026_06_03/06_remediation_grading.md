# Phase 6: Remediation Grading

## Classification

- **Grade A**: Must fix before production — blocks core functionality, causes data loss, or is a security vulnerability
- **Grade B**: Should fix in next iteration — degrades experience or creates risk but has workarounds
- **Grade C**: Nice to fix — cosmetic, performance optimization, or future risk

---

## Grade A: Must Fix Before Production (16 issues)

| # | Issue | Agent | Effort | Fix Strategy |
|---|---|---|---|---|
| A1 | Compatible Anthropic path sends flat strings (400 errors) | 04 | M | Rewrite `compatible_provider.rs:144-148` to produce structured content blocks |
| A2 | `shell.command` hidden from manifest under ReadOnly | 07 | S | Always include all tools in manifest; move control to PermissionPolicy |
| A3 | `file.write`, `file.edit`, `patch.apply` hidden in ReadOnly | 07 | S | Same fix as A2 |
| A4 | Model repeats same validation error 8+ times without adaptation | 16 | M | Add max-consecutive-identical-error limit; stop after 3 |
| A5 | Runtime panic: UTF-8 `is_char_boundary` assertion | 16 | S | Fix string truncation to use `floor_char_boundary` |
| A6 | Same-tool-error plateau takes 70 iterations to detect | 16 | S | Reduce plateau threshold from ~70 to ~10 for same-error patterns |
| A7 | `model.call_blocked` HTTP 400 infinite retry | 16 | M | Add exponential backoff; fail-fast after N consecutive 400s |
| A8 | Compaction in-process only (no Flash model) | 08 | L | Add optional Flash model call or document decision to stay in-process |
| A9 | No L1 state object (CompactionSummary is markdown blob) | 08 | L | Implement serializable state object for compact state representation |
| A10 | Compaction irreversible (model can't see old events) | 08 | L | Preserve reconstruction path; scope decision with product |
| A11 | Classifier "Deny" is not a true deny (user-overridable) | 14 | S | Map `SafetyCheck { classifier_approvable: false }` to `PermissionResolution::Deny` |
| A12 | TOCTOU race: cancel cleanup clobbers new turn | 10 | M | Add generation counter to `active_turns`; validate epoch on release |
| A13 | Unknown tool hallucination (model calls non-existent tools) | 16 | M | Add tool existence validation before execution; improve error feedback |
| A14 | DSML leak fallback failures (visible_finalizer.failed) | 16 | M | Fix DSML detection edge cases; improve fallback text quality |
| A15 | Prompt keywords drive tool exposure (TurnRoute chain) | 07 | M | Make manifest stable; remove TurnRoute→exposure mapping |
| A16 | Concurrent path bypasses TCML (5 of 7 stages skipped) | 06 | L | Replace ad-hoc `parse_tool_arguments` with full `mediate_tool_call` |

**Grade A total effort: 4S + 8M + 4L = ~16 engineer-weeks**

---

## Grade B: Should Fix in Next Iteration (22 issues)

| # | Issue | Agent | Effort | Fix Strategy |
|---|---|---|---|---|
| B1 | 6 loop owners with 23 exit paths | 01 | L | Consolidate into NativeLoopTurnController as single stop authority |
| B2 | Mid-loop manifest change via EscalateToCodeEdit | 01 | M | Make EscalateToCodeEdit a new turn, not mid-loop change |
| B3 | Plan approval resume race | 02 | M | Add synchronization around synthetic artifact insertion |
| B4 | Permission resume race: unlocked decide→execute window | 02 | M | Make decide_permission + execute_tool atomic |
| B5 | `pending_native_decision` not cleared on cancel | 02 | S | Clear pending decision in cancel_session |
| B6 | Permission_id → tool_call_id via string suffix stripping | 03 | M | Add explicit mapping table instead of string manipulation |
| B7 | Dirty streaming events on dual-protocol 400 fallback | 04 | M | Gate stream_event_handler on live_stream_status_is_success |
| B8 | Legacy history Anthropic projection loses tool metadata | 04 | M | Preserve function names and arguments in legacy path |
| B9 | `model_compatibility` field never enforced | 07 | M | Enforce model_compatibility in allow_tool_for_manifest |
| B10 | `tui_fastauto` separate inconsistent tool filtering | 07 | M | Unify with main manifest builder |
| B11 | Merge `_loop_` suffix omits permission_id/plan_approval_id | 03 | S | Add to REWRITABLE_ID_KEYS |
| B12 | `provider_tool_call_id` may be synthetic, indistinguishable | 03 | S | Tag synthetic IDs or always store provider origin |
| B13 | Dual reasoning_content injection in Anthropic path | 04 | S | Remove non-standard `reasoning_content` field; keep thinking block only |
| B14 | Orphan tool result risk in conversation projection | 04 | M | Add tool_call_id cross-validation |
| B15 | Error flag uses fragile substring match `"is_error":true` | 04 | S | Parse as structured JSON field |
| B16 | Concurrent path uses raw tool IDs (alias lookup fails) | 06 | S | Use canonical ID from alias registry; fix with A16 |
| B17 | Qwen `max_active_tools` silently truncates catalog | 07 | M | Add warning or error when tools are dropped due to budget |
| B18 | reasoning_content reduced to 240-char preview | 08 | L | Preserve full reasoning_content in ReasoningReplayManager |
| B19 | Python backend telemetry black hole (10 events missing) | 16 | L | Add missing event types to Python sidecar |
| B20 | Token estimation naive (chars/4 vs word_count) | 08 | M | Use proper tokenizer or tiktoken-based estimation |
| B21 | "Stop flash" — 250ms interrupt polling gap | 10 | M | Reduce polling to 50ms; add interrupt check in observer |
| B22 | `suppressNextCallCompletedSettleRef` leaks across sessions | 12 | S | Reset on `session.state_changed` and `model.call_started` |

**Grade B total effort: 8S + 12M + 3L = ~22 engineer-weeks**

---

## Grade C: Nice to Fix (22 issues)

| # | Issue | Agent | Effort |
|---|---|---|---|
| C1 | Stage order cosmetic mismatch in TCML event declaration | 06 | XS |
| C2 | "below_threshold" telemetry noise | 08 | XS |
| C3 | `ledger_tool_call_id` ≠ `tool_call_id` format confusion | 03 | S |
| C4 | `--force` false positives in DENY_SUBSTRINGS | 14 | XS |
| C5 | TurnBudget hard cap fields retained (future misuse risk) | 15 | XS |
| C6 | Qwen sidecar errors silent (exit status 1 → Completed) | 16 | S |
| C7 | Missing dangerous programs: `mkfs`, `dd`, `fdisk`, `systemctl` | 14 | S |
| C8 | `rmdir` not blocked by either security layer | 14 | XS |
| C9 | `sudo` not in Layer A hard block | 14 | XS |
| C10 | PermissionService creates new gate per evaluation | 14 | M |
| C11 | No explicit task-phase tracking | 09 | L |
| C12 | No endurance/large-scale tests (≥1000 events) | 13 | M |
| C13 | No large tool output tests (>100KB) | 13 | S |
| C14 | No subagent lifecycle integration test | 13 | M |
| C15 | No identity chain fixture data | 13 | M |
| C16 | No scripted AgentKernel test for shell recovery | 13 | M |
| C17 | No scripted AgentKernel test for plan approval resume | 13 | M |
| C18 | No end-to-end TCML pipeline integration test | 13 | M |
| C19 | Narrative text swallowed via terminalStreamClosedRef | 12 | M |
| C20 | Markdown re-renders entire accumulated text per frame | 12 | M |
| C21 | No "Thinking" run status in status bar | 12 | S |
| C22 | Recoverable tool failures hidden from user | 12 | S |

**Grade C total effort: 6XS + 9S + 8M + 1L = ~13 engineer-weeks**

---

## Summary

| Grade | Count | Est. Effort | Timeline |
|---|---|---|---|
| **A** (Must fix) | 16 | ~16 eng-weeks | Before production launch |
| **B** (Should fix) | 22 | ~22 eng-weeks | First 2 iterations post-launch |
| **C** (Nice to fix) | 22 | ~13 eng-weeks | Backlog, prioritize by user impact |
| **Total** | **60** | **~51 eng-weeks** | |

## Quick Wins (< 1 day each, 15 items)

These Grade A/B issues can be fixed in under a day:

1. **A2/A3**: Include all tools in manifest (change `allow_tool_for_manifest` filter)
2. **A5**: Fix UTF-8 string boundary with `floor_char_boundary()`
3. **A6**: Reduce plateau threshold constant
4. **A11**: Change `SafetyCheck` → `PermissionResolution::Deny` mapping
5. **B5**: Clear `pending_native_decision` in `cancel_session`
6. **B11**: Add `permission_id`, `plan_approval_id` to `REWRITABLE_ID_KEYS`
7. **B12**: Tag synthetic provider_tool_call_id values
8. **B13**: Remove non-standard `reasoning_content` from Anthropic builder
9. **B15**: Replace substring `is_error` check with JSON field parse
10. **B16**: Use canonical tool ID in concurrent path
11. **B22**: Reset `suppressNextCallCompletedSettleRef` on session boundaries
12. **C1**: Fix TCML stage order in event declaration
13. **C2**: Suppress `below_threshold` for trivially small requests
14. **C8**: Add `rmdir` to program deny list
15. **C9**: Add `sudo` to Layer A hard block

**Quick wins total: ~5 engineer-days for 15 fixes across all grades.**
