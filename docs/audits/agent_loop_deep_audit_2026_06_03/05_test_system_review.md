# Phase 5: Test System Review

## Overview

The test system has three tiers: deterministic Rust tests, scripted provider integration tests, and live API canaries. Coverage is strong for tool execution and repair safety, but has critical gaps in integration scenarios and endurance testing.

## Test Coverage Matrix

### Tier 1: Deterministic Rust Tests

| Test Suite | Count | Covers | Gaps |
|---|---|---|---|
| `tool_harness.rs` | 27 | Tool execution, sensitive paths, permission gating, edit validation, patch staleness, LSP stubs, plan governance | No shell external decision recovery |
| `harness.rs` | 10 | Coding without model, failure repair, recorded loops, permission boundaries, external decision resume | No shell permission, no plan approval resume, no subagent |
| `native_agent_loop_tests.rs` | ~90 | Prompt stripping, route classification, tool exposure, permission resolution, stream parsing, dedup, budget behavior, HTTP 400 recovery | No TCML end-to-end, no endurance |
| `test_runtime_event_replay.mjs` | 1 | 29 hardcoded events through full reducer pipeline | Only 29 events; small payloads |
| `test_progress_ledger.mjs` | 1 | Progress ledger unit tests | Isolated; no integration with event flow |

### Tier 2: Scripted Provider Integration

| Fixture | Covers | Gap |
|---|---|---|
| `external_block_fixture` | file_write block boundary | Shell command not tested |
| `external_resume_fixture` | file_write external decision → resume | Shell not tested |
| `provided_permission_fixture` | Inline file_write permission | No external decision path |
| `plan_enter_fixture` | plan_enter tool routing | No approval → resume tested |
| `coding_no_model` | No-model coding cycle | Basic only |
| `failure_repair` | Command failure + repair | Single failure, not repeated loops |

### Tier 3: Live API Canaries

| Canary | What It Tests | What It CAN'T Test |
|---|---|---|
| `gui_argon_longtask_stress.mjs` | End-to-end model → UI flow | Deterministic sequences, permission edge cases |
| `live_deepseek_smoke.py` | API connectivity | Error recovery, specific tool patterns |
| `run_live_native_eval.py` | Native evaluation | Deterministic assertions |
| `deepseek-sidecar-live-smoke` | Sidecar connectivity | Specific event sequences |

## Critical Test Gaps

### Gap 1: Shell Command External Decision Recovery (P1)
**What:** No scripted transport triggers `shell.command` → block → external approval → resume shell execution.
**Why it matters:** Shell is the most dangerous tool. Its recovery path is untested at `AgentKernel::run_turn` level.
**Existing coverage:** `tool_harness.rs` unit tests (no resume); `gui_permission_longtask_smoke.mjs` (mock HTTP, not AgentKernel).

### Gap 2: Plan Approval → Resume Integration (P1)
**What:** No scripted AgentKernel test for: plan_enter → WaitingForPlanApproval → approve → model continuation → more tools.
**Why it matters:** The only test is `gui_full_stack_regression.mjs` which uses mock HTTP runtime, not AgentKernel.
**Existing coverage:** `plan_enter_fixture` tests input only; no post-approval continuation test.

### Gap 3: End-to-End TCML Pipeline (P1)
**What:** Each TCML stage tested in isolation. No single test chains: alias → schema → repair → permission → dispatch → result.
**Why it matters:** A regression in stage ordering or inter-stage data flow wouldn't be caught by isolated tests.
**Existing coverage:** Individual unit tests for each stage; live API canary (non-deterministic).

### Gap 4: Endurance/Large-Scale (P2)
**What:** No test with ≥1000 events. Maximum scripted tool calls: 48.
**Why it matters:** UI virtualization, memory growth, dedup buffer overflow not validated.
**Existing coverage:** None.

### Gap 5: Large Tool Output (P2)
**What:** All test tool outputs are <10KB. No MB-scale payloads tested.
**Why it matters:** 10MB shell output could freeze UI or trigger OOM.
**Existing coverage:** None.

### Gap 6: Subagent Lifecycle (P2)
**What:** No scripted transport uses `task.dispatch` with `subagent.spawned` → `subagent.summary_recorded` → `subagent.completed`.
**Why it matters:** Subagent event sequence corruption would go undetected until production.
**Existing coverage:** `gui_full_stack_regression.mjs` round 4 (mock only).

### Gap 7: Identity Chain Fixtures (P2)
**What:** No fixture data mapping known input → expected TCML pipeline output.
**Why it matters:** Identity chain regressions can't be caught by existing fixtures.
**Existing coverage:** Parser golden data stops at alias level.

## What IS Well Tested

| Area | Assessment |
|---|---|
| Tool execution (27 harness tests) | Strong — all major tools, edge cases |
| Repair safety (content/command never repaired) | Strong — confirmed by tests |
| Permission gating (Allow/Ask/Deny paths) | Strong — all modes covered |
| Stream parsing (chunk assembly) | Strong — multi-chunk, partial markup |
| HTTP 400 recovery (3 tests) | Adequate — dual protocol fallback |
| DSML fallback | Adequate — leak detection and recovery |
| Manifest tool filtering | Strong — all exposure modes |
| Visible final answer heuristic | Strong — transition detection, negative filter |
| Route classification | Strong — keyword → exposure mapping |

## Recommended Test Additions (Priority Order)

1. `run_scripted_native_agent_loop_v2_shell_permission_fixture` — shell.command block + external resume
2. `run_scripted_native_agent_loop_v2_plan_approval_fixture` — plan enter + approve + continue
3. `run_scripted_native_agent_loop_v2_tcml_pipeline_fixture` — full alias→schema→repair→permission→dispatch chain
4. `gui_full_stack_regression.mjs --tool-calls=1000` — endurance mode
5. `eval/fixtures/identity_chain/` — full pipeline input→expected output records
6. Large tool output event in `test_runtime_event_replay.mjs`
7. Subagent lifecycle scripted transport fixture
