# Upgrade Plan 2026-05-18 P1/P2 Execution Plan

Source contract:

- `/Users/gongyuxuan/Documents/deep-code/upgrade_plan_2026_05_18.md`

Scope:

- Finish the P1/P2 atomic repair list from the source contract.
- Keep broader Phase 1-4 month-scale extractions out of this slice unless an
  atomic P1/P2 item requires a small enabling interface.
- Preserve DeepSeek/Qwen native separation and existing event replay contracts.

Denied in this slice:

- deleting legacy `tool_call_parser.rs` / `tool_contract.rs`;
- full TurnController extraction;
- full TCML migration;
- provider-layer redesign;
- destructive git operations;
- dependency installs or network calls.

## Completion Evidence

1. P1-1 streamed/parsed mismatch: completed.
   - detect parsed tool calls that did not produce streamed execution records;
   - append a model-readable synthetic tool result for each missing parsed call;
   - preserve provider tool call id when available;
   - test that mismatch no longer leaves an unpaired tool call.

2. P1-2 PermissionResolver production status: completed.
   - verify production uses the unified resolver;
   - production `tool_permission_decision` routes through `PermissionResolver`;
   - update resolver wording to the request-shaped mode fallback;
   - add/keep regression checks around resolver path.

3. P1-3 / P1-4 facade-only markers: completed.
   - explicitly mark `agent_kernel::turn_controller` and `AgentKernel` as
     compatibility facades until Phase 1 extraction lands.

4. P1-5 TCML legacy scatter: completed for this atomic slice.
   - document current non-migration boundary in code comments;
   - add tests around the still-legacy stream/parse handoff so mismatches are
     visible until Phase 2 migrates the full pipeline.

5. P1-6 TurnBudget drift: completed.
   - add a TurnBudget-aware context budget allocator;
   - wire native loop budgeting through `TurnState::budget`;
   - test that max input/output/reasoning settings shape the runtime budget.

6. P2-1 PermissionPolicy signature: completed.
   - replace `evaluate(mode, tool_id)` with a request-shaped evaluation object;
   - include args, request type, session id, and command summary in the public
     contract;
   - keep mode fallback behavior stable.

7. P2-2 DSML false positives: completed.
   - stop using raw string contains as the sole executable DSML detector;
   - ignore DSML-like text inside fenced code/discussion unless it contains
     executable markup outside fences;
   - test discussion/fenced examples do not trigger fallback parse events while
     real DSML markup still does.

## Verification

Focused checks:

- `cargo test -p researchcode-runtime --lib permission_policy -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_streamed_parsed_mismatch -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib dsml -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib context_budget -- --test-threads=1`
- `cargo test -p researchcode-runtime --lib native_agent_loop_v2_executes_deepseek_dsml_fallback_tools -- --test-threads=1`

Final checks:

- `cargo fmt --check`
- `cargo test -p researchcode-runtime --lib -- --test-threads=1`
- `cargo test -p researchcode-kernel --lib -- --test-threads=1`

Observed final result:

- `cargo fmt --check`: passed.
- stale permission-mode grep: passed, no `NativeAgentPermissionMode`,
  `native_permission_mode`, or old `PermissionPolicy::evaluate(mode, tool_id)`
  call remains.
- `cargo test -p researchcode-kernel --lib -- --test-threads=1`: passed,
  27 tests.
- `cargo test -p researchcode-runtime --lib -- --test-threads=1`: passed,
  550 tests. This was run outside the sandbox because local API server tests
  bind `127.0.0.1`.
