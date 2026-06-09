# Issue Matrix — deep-code Agent Loop Audit

## Severity Distribution

| Severity | Count | Description |
|---|---|---|
| P0 | 12 | Data loss, security, crashes, malformed requests, API blocking |
| P1 | 18 | Broken features, missing integration coverage, races |
| P2 | 20 | Degraded behavior, performance, coverage gaps |
| P3 | 10 | Cosmetic, documentation, minor inefficiencies |

---

## P0 Issues

| # | Agent | Issue | Files | doc39 § |
|---|---|---|---|---|
| P0-01 | 04 | Generic compatible Anthropic path sends flat strings instead of structured content blocks — **any tool call gets 400** | `compatible_provider.rs:144-148` | §2.4 |
| P0-02 | 07 | `shell.command` hidden from manifest under ReadOnly exposure — **direct doc39 violation** | `manifest.rs:66-97` | §2.3 |
| P0-03 | 07 | `file.write`, `file.edit`, `patch.apply` hidden in ReadOnly — **direct doc39 violation** | `manifest.rs:66-97` | §2.3 |
| P0-04 | 16 | `model.call_blocked` by HTTP 400 from DeepSeek API — 15 occurrences, infinite retry pattern | `native_agent_loop_model_io.rs` | §7.1 |
| P0-05 | 16 | Runtime panic: `assertion failed: self.is_char_boundary(new_len)` — UTF-8 string boundary crash | string handling in session/event_log | — |
| P0-06 | 16 | Model repeats same `file.edit` validation error 8+ times without adaptation | `native_agent_loop_tools.rs`, TCML recovery | §2.4 |
| P0-07 | 16 | DSML leak fallback — `agent.visible_finalizer.failed` with `http_status_400` / `empty_visible_response` | `native_profile/deepseek/stream.rs` | §5.2 |
| P0-08 | 16 | Unknown tool hallucination — model calls `create_file`, `memory_get_all` (not real tools) | `tool_execution.rs`, tool registry | §2.1 |
| P0-09 | 16 | Same-tool-error plateau takes 70 iterations to detect and stop | `turn_controller.rs`, plateau detection | §7.1 |
| P0-10 | 08 | Compaction is entirely in-process — no Flash model role (doc39 §12 deviation) | `agent_kernel/compactor.rs` | §12 |
| P0-11 | 08 | No L1 state object — `CompactionSummary` is a markdown blob | `compaction.rs` | §4 |
| P0-12 | 08 | Compaction is irreversible — model cannot "see" old events after compaction | `compactor.rs`, `event_log.rs` | §5 |

---

## P1 Issues

| # | Agent | Issue | Files | doc39 § |
|---|---|---|---|---|
| P1-01 | 01 | 6 distinct loop owners create fragmented stop authority | `native_agent_loop.rs` | §7 |
| P1-02 | 01 | `EscalateToCodeEdit` changes manifest mid-loop without explicit signal | `native_agent_loop.rs:2814-2837` | §2.1 |
| P1-03 | 02 | Plan approval resume race: synthetic artifact insertion may race with concurrent loop continuation | `runtime_facade_impl.rs` | §4.5 |
| P1-04 | 02 | Permission resume race: unlocked window between `decide_permission()` and `execute_tool()` | `runtime_facade_impl.rs` | §7.2 |
| P1-05 | 02 | `pending_native_decision` not cleared on cancel — stale state on next turn | `session.rs` | §4.5.1 |
| P1-06 | 03 | Permission_id → tool_call_id reverse lookup via string suffix stripping (fragile) | `runtime_facade_impl.rs:637-639` | §4.2 |
| P1-07 | 04 | Dirty streaming events on dual-protocol 400 fallback — content block events leak from failed Anthropic attempt | `native_agent_loop_continuation.rs:413-421` | §3.1 |
| P1-08 | 04 | Legacy history Anthropic projection loses tool metadata | `conversation_history.rs:91-106` | §4.2 |
| P1-09 | 06 | Concurrent read-only tool path bypasses TCML entirely — loses relational defaults, repair, aliases | `native_agent_loop.rs:1719-1789` | §5, §7 |
| P1-10 | 07 | Mid-loop manifest change via `EscalateToCodeEdit` — model sees new tools without policy-change signal | `native_agent_loop.rs:2814-2837` | §2.1 |
| P1-11 | 07 | `model_compatibility` field defined but never enforced | `manifest.rs`, `kernel/src/tool.rs` | §3.1 |
| P1-12 | 07 | `tui_fastauto` maintains separate inconsistent tool filtering | `kernel/src/tool.rs:684-696` | §2.3 |
| P1-13 | 10 | TOCTOU race between `cancel_session` and `runtime_submit_user_message` — active turn clobbered by old task | `desktop/src-tauri/src/main.rs:1382-1386` | §7.2 |
| P1-14 | 13 | No scripted AgentKernel test for shell command external decision recovery | `native_agent_loop_fixtures.rs` | — |
| P1-15 | 13 | No scripted AgentKernel test for plan approval → resume cycle | `native_agent_loop_fixtures.rs` | Plan section |
| P1-16 | 13 | No end-to-end TCML pipeline integration test (all stages chained) | `harness.rs`, `native_agent_loop_tests.rs` | §5 |
| P1-17 | 14 | Classifier `Deny` is not a true deny — becomes user-approvable `SafetyCheck` → `Ask` | `permission_gate.rs:384-387` | §1.8 |
| P1-18 | 16 | Python backend telemetry black hole — 10 event types missing (cache, compaction, tool part) | Python sidecar | §6 |

---

## P2 Issues

| # | Agent | Issue | Files | doc39 § |
|---|---|---|---|---|
| P2-01 | 03 | Merge-time `_loop_` suffix rewrite omits `permission_id` and `plan_approval_id` | `session.rs:1125-1131` | §4.2 |
| P2-02 | 03 | `provider_tool_call_id` may be synthetic (not real provider ID), indistinguishable later | `native_agent_loop_tools.rs:138-146` | §4.2 |
| P2-03 | 04 | Dual `reasoning_content` injection in Anthropic path (thinking block + non-standard field) | `live_model_request.rs` | §2.3 |
| P2-04 | 04 | Orphan tool result risk — `conversation_messages_from_event_log` emits tool results without cross-validation | `conversation_history.rs` | §4.2 |
| P2-05 | 04 | Error flag uses fragile substring match `contains("\"is_error\":true")` | `native_agent_loop_continuation.rs:395-404` | — |
| P2-06 | 06 | Concurrent path uses raw tool IDs (not normalized) — alias tools fail lookup | `native_agent_loop.rs:1719-1789` | §5 |
| P2-07 | 07 | Prompt keywords drive tool exposure via `TurnRouter` → `TurnRoute` → exposure chain | `turn_router.rs:6-57`, `prompt.rs:198-229` | §2.4 |
| P2-08 | 07 | Qwen `max_active_tools` silently truncates tool catalog | `prompt_assembler.rs:325` | §2.3 |
| P2-09 | 08 | `reasoning_content` reduced to 240-char preview — cannot support actual replay | `reasoning.rs:149-234` | §6 |
| P2-10 | 08 | Token estimation naive (chars/4 vs word_count) — may underestimate for structured JSON | `context_budget.rs` | §13 |
| P2-11 | 09 | No explicit task-phase tracking (EvidenceCollected, WriteExpected, etc.) | `turn_state.rs`, `evidence_ledger.rs` | §8 |
| P2-12 | 10 | "Stop flash" — 250ms interrupt polling gap during HTTP streaming | `sidecar_http_transport.rs:305-319` | §7.2 |
| P2-13 | 12 | Narrative text swallowed via `terminalStreamClosedRef` after tool completion | `useRuntimeEventApplication.ts:197-206` | §4.3.1 |
| P2-14 | 12 | `suppressNextCallCompletedSettleRef` leaks across sessions | `useRuntimeEventApplication.ts:127,194-196` | §4.4 |
| P2-15 | 13 | No endurance/large-scale tests (≥1000 events) | `gui_*.mjs`, `harness.rs` | — |
| P2-16 | 14 | `rmdir` not blocked by either security layer | `permission_gate.rs`, `runtime_facade_impl.rs` | §1.8 |
| P2-17 | 14 | `sudo` not in Layer A hard block — sudo commands user-approvable | `runtime_facade_impl.rs:3192-3209` | §1.8 |
| P2-18 | 14 | `PermissionService` creates new gate per evaluation — breaks long-lived gate design | `permission_service.rs:39-53` | §1.8 |
| P2-19 | 14 | Missing dangerous programs: `mkfs`, `dd`, `fdisk`, `systemctl`, etc. | `permission_gate.rs:192` | §1.8 |
| P2-20 | 16 | Convergence loop not self-terminating — votes "continue" despite detecting non-progress | `convergence_enforcer.rs` | §7.1 |

---

## P3 Issues

| # | Agent | Issue | Files |
|---|---|---|---|
| P3-01 | 03 | `ledger_tool_call_id` vs `tool_call_id` have different formats for same logical call — confusion risk | `native_agent_loop.rs:1890` |
| P3-02 | 06 | Stage order cosmetic mismatch in TCML event declaration vs execution | `contract.rs` |
| P3-03 | 08 | "below_threshold" telemetry noise — emitted for every model call under 192K | `native_turn_controller.rs` |
| P3-04 | 10 | Active turn released on ALL paths but no generation counter for epoch check | `desktop/src-tauri/src/main.rs` |
| P3-05 | 12 | Markdown re-renders entire accumulated text per animation frame | `useStreamingTranscript.ts:169-182` |
| P3-06 | 12 | Recoverable tool failures hidden from user — silently promoted to "completed" | `useRuntimeEventApplication.ts:66-96` |
| P3-07 | 12 | No "Thinking" run status — thinking only in transcript body, not status bar | `runtimeEventViewModel.ts:18-27` |
| P3-08 | 14 | `--force` false positives in DENY_SUBSTRINGS | `permission_gate.rs:35-57` |
| P3-09 | 15 | `TurnBudget` hard cap fields retained — future misuse risk | `turn_state.rs:27-33` |
| P3-10 | 16 | Qwen sidecar errors silent — `sidecar_failed: exit status 1` but session reaches Completed | `sidecar_http_transport.rs` |

---

## Positive Findings (P0 ✓)

| # | Agent | Finding |
|---|---|---|
| ✓1 | 01 | Tool-result-to-model path ALWAYS followed — no skip paths exist |
| ✓2 | 02 | Plan approval uses synthetic artifact + new prompt round (correct model) |
| ✓3 | 06 | `file.write.content` and `shell.command.command` NEVER repaired (doc39 compliant) |
| ✓4 | 06 | Schema errors returned as recoverable observations with `retryable: true` |
| ✓5 | 08 | Compaction threshold correctly DeepSeek-aware (192K) |
| ✓6 | 15 | All rejected patterns removed from production: `final_answer`, `disable_tools`, `disable_tools_and_request_final_answer`, `model_continuation_skipped` |
| ✓7 | 15 | Current code explicitly uses `stop_without_disabling_tools` and `stop_with_structured_failure` |
| ✓8 | 16 | Tool call identity chain PERFECT across 40 sessions (2,941 tool calls, zero orphans) |
| ✓9 | 14 | Commands NEVER executed via shell — `std::process::Command` with tokenization |
| ✓10 | 14 | Path traversal protection comprehensive for file tools |
| ✓11 | 16 | Permission resume flow works correctly — all completions have `ok: true` |
| ✓12 | 11 | UTF-8 multi-byte character handling safe at event boundaries |
