# Agent 13: Harness / Test System Audit

## Conclusion

The test suite has solid deterministic coverage for tool execution, permission gating, and TCML repair safety. However, critical integration gaps exist for shell command external decision recovery, plan approval → resume at AgentKernel level, full identity chain coverage, and large-scale endurance/stress testing.

**Severity:** P1 (missing AgentKernel-level integration tests for shell and plan approval recovery paths)

## Files Involved

- `crates/runtime/src/tool_harness.rs` — 27 deterministic tool execution tests
- `crates/runtime/src/harness.rs` — 10 scripted transport aggregate cases
- `crates/runtime/src/native_agent_loop_tests.rs` — ~90 unit tests (4515 lines)
- `crates/runtime/src/native_agent_loop_fixtures.rs` — scripted transport fixtures
- `desktop/gui_*.mjs` — 6 GUI smoke/stress tests
- `desktop/test_runtime_event_replay.mjs` — 29-event replay test
- `desktop/test_progress_ledger.mjs` — progress ledger unit tests
- `eval/fixtures/` — parser golden data, shell permission cases, patch scenarios

## Key Findings

### Finding 1: No scripted AgentKernel test for shell external decision recovery (P1)

`external_resume_fixture` and `external_block_fixture` only test `file_write`. No scripted transport triggers `shell.command` → block → external approval → resume shell execution at `AgentKernel::run_turn` level. GUI smoke tests use mock HTTP runtime, not actual AgentKernel.

### Finding 2: No scripted AgentKernel test for plan approval → resume (P1)

`gui_full_stack_regression.mjs` tests plan approval UI via mock runtime. Actual `AgentKernel::run_turn` path (plan approved → model receives continue prompt → more tool calls) is not tested with scripted transport. Only `plan_enter` input tested, not approval + continue cycle.

### Finding 3: No end-to-end TCML pipeline integration test (P1)

Each TCML stage (alias, schema, repair, permission, dispatch) tested in isolation. No single scripted provider session chains all stages. Live API canary is the only end-to-end test but is non-deterministic.

### Finding 4: No endurance/large-scale tests (P2)

Maximum scripted tool calls: 48. No tests with ≥1000 events. UI virtualization, memory usage, and long-task stability untested at scale.

### Finding 5: No large tool output tests (P2)

All test tool outputs are <10KB. No tests inject >100KB tool result payloads to verify UI rendering performance, Markdown parser stability, or memory limits.

### Finding 6: No subagent integration test in scripted transport (P2)

`harness.rs` has no subagent cases. No scripted native agent loop fixture uses `task.dispatch` response with subagent lifecycle events.

### Finding 7: No identity chain fixture data (P2)

No fixture directory contains known input → expected TCML pipeline output records. Parser golden data stops at alias resolution; no manifest → repair → permission → dispatch fixture data.

### Finding 8: Deterministic test coverage is strong (P0 ✓)

- 27 tool harness tests: sensitive path denial, escape denial, permission gating, edit validation, patch staleness, plan governance
- 10 harness aggregate cases: coding_no_model, failure_repair, recorded loops, permission boundaries
- ~90 unit tests in native_agent_loop_tests.rs: prompt stripping, route classification, tool exposure, permission resolution, stream parsing, dedup detection, budget behavior
- Repair safety confirmed: file.write.content and shell.command.command NEVER repaired

### Finding 9: Live API canary coverage (P0 ✓)

Live tests verify model connectivity, streaming, and basic tool flow. `assertNoArchitecturalDrift` checks doc39 compliance. Cannot verify deterministic event sequences, permission edge cases, or error recovery paths.

## doc39 Conflict

- **Yes** (Plan Approval section): Claims agent continues after plan approval but no deterministic test verifies this
- **No** for repair safety, manifest building, and permission gating coverage

## Suggested Fix

1. Add `run_scripted_native_agent_loop_v2_shell_permission_fixture` with scripted transport triggering shell.command block + external resume
2. Add `run_scripted_native_agent_loop_v2_plan_approval_fixture` with plan_enter → approval → post-approval tool continuation
3. Add `run_scripted_native_agent_loop_v2_tcml_pipeline_fixture` with chain covering alias → schema → repair → permission → dispatch
4. Extend `gui_full_stack_regression.mjs` with `--tool-calls=1000` endurance mode
5. Create `eval/fixtures/identity_chain/` directory with full prompt → expected pipeline output records
6. Add subagent lifecycle scripted transport fixture

## Handoff Needed

- Testing team: implement the missing integration fixtures
- GUI team: confirm mock runtime tests adequately cover AgentKernel integration
