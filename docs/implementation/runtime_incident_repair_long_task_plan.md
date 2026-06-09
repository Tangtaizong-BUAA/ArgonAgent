# Runtime Incident Repair Long Task Plan

## TaskContract

Goal: repair the long-session runtime failure mode captured by
`.researchcode/runtime_desktop/runtime_session_1778982897464665000/events/runtime_events.jsonl`
and make it reproducible through GUI/runtime engineering verification.

Scope:
- Native DeepSeek/Qwen runtime loop.
- TCML/tool mediation recovery behavior.
- Permission approval and resume identity/state.
- Continuation prompt audit metadata.
- Desktop GUI event consumption checks.
- Incident verification scripts and focused tests.

Allowed paths:
- `crates/runtime/src/**`
- `crates/runtime/tests/**`
- `desktop/**`
- `scripts/**`
- `docs/implementation/**`
- local fixture and generated test artifact directories.

Denied paths:
- secrets, private keys, SSH keys, `.env` contents.
- unrelated user files outside the workspace.
- git history rewrites or destructive cleanup.
- dependency lockfile churn unless directly required by verification.

Model policy:
- Preserve DeepSeek/Qwen native optimization.
- Do not mark compatible providers as native.
- Any native optimization change must be backed by a focused test or fixture check.

Shell/network/package policy:
- Use non-destructive local checks.
- No package installs unless explicitly approved.
- Live provider GUI checks are allowed as opt-in verification, not as mandatory automated CI.

Stop conditions:
- A permission/security contract contradiction appears.
- A required dependency install or network call is unavailable.
- Focused tests expose a blocker that would require changing scope.

Final report format:
- Changed files.
- Summary.
- Tests/checks run.
- Risks.
- Unresolved questions.
- Next recommended task.

## Repair Workstream

1. Verification baseline
   - Keep `desktop/gui_three_round_smoke.mjs --incident-verify`.
   - Use `npm run gui:incident-fixture` to prove the detector catches the pre-doc39 production failure.
   - Use `npm run gui:incident-live` as the real GUI/Rust/runtime acceptance probe.

2. Recovery escalation
   - Track repeated tool contract failures by normalized signature:
     `resolved tool + error_code + missing fields`.
   - After the third repeated signature, emit `agent.recovery.escalated`.
   - Mark the model-readable error as non-retryable and include concrete next-action guidance.
   - Special-case `file.edit` missing `old_string/new_string/base_hash` with guidance to read the file first or switch to `file.write` for new/whole-file writes.

3. Permission identity and state
   - Unify the ID seen by live GUI events, merged event logs, and `pending_native_decision`.
   - Ensure permission state is replayable and synchronized after event merge.
   - Reject or clear stale GUI approval cards when runtime reports no pending permission.

4. Shell permission semantics
   - Ensure `permission_required:true` tools enter approval flow.
   - Prevent `PermissionRequired("shell.command")` from becoming a normal tool result/recovery loop.

5. Continuation audit hashes
   - Fill `prompt_hash`, `tool_catalog_hash`, and prompt token estimates for continuation calls.
   - Allow `unknown` only with an explicit unavailable reason.

6. Production path telemetry
   - Verify desktop/native path emits role, compaction, cache, partial tool call, DSML leak, and flash-savings telemetry.
   - Keep strict telemetry optional until every emitter is wired.

7. Long-session GUI performance
   - Treat dense `model.stream_delta` streams as a warning signal.
   - Add aggregation/virtualization separately after correctness fixes.

## Acceptance Checks

- `npm run gui:incident-fixture` passes and reports the known bad baseline.
- Focused Rust tests cover repeated contract-error escalation.
- A live approval probe no longer reports stale `no pending permission`.
- `shell.command` permission no longer escapes as a tool result.
- Continuation model calls are auditable.

## Execution Ledger

Completed in the first implementation slice:

- Adapted `desktop/gui_three_round_smoke.mjs` into a runtime incident verifier.
- Added fixture-only and live incident commands in `desktop/package.json`.
- Documented the incident verification workflow in `desktop/README.md`.
- Added progressive recovery escalation for repeated tool-contract rejection.
- Preserved permission and plan-approval IDs across session event merge, and replayed merged state side effects into in-memory pending state.
- Routed `shell.command` through permission preflight before execution, while preserving safe directory-list alias recovery.
- Filled continuation `prompt_hash`, `tool_catalog_hash`, and prompt token estimates for native model calls.
- Emitted production-path role and compactor telemetry without changing model routing.
- Emitted DeepSeek parser telemetry for DSML leak recovery, streaming partial tool calls, and assembled tool calls.
- Activated DeepSeek cache-zone wrapping in the native loop production prompt path.
- Added runtime prefix-reuse telemetry for `deepseek.cache.zone_a/b.hit/miss`, and taught the telemetry aggregator to count those events.
- Unified native turn ledger IDs with the current `session.turn_started` turn ID when running through the runtime facade.
- Routed command-classifier hard blocks through forced safety-ask/recoverable model-readable feedback without allowing blocked shell execution.
- Upgraded DeepSeek preflight compaction from cosmetic telemetry to a compacted-request rebuild path for oversized initial native-loop requests.

Verified commands:

- `cargo test -p researchcode-runtime native_agent_loop_v2_ -- --nocapture`
- `cargo test -p researchcode-runtime native_turn_controller::tests:: -- --nocapture`
- `cargo test -p researchcode-runtime session::tests:: -- --nocapture`
- `cargo fmt --check`
- `node --check gui_three_round_smoke.mjs`
- `npm run gui:incident-fixture`
- `cargo test -p researchcode-runtime agent_kernel::telemetry::tests:: -- --nocapture`
- `cargo test -p researchcode-runtime native_agent_loop_v2_rebuilds_request_after_preflight_compaction -- --nocapture`

Remaining acceptance boundary:

- `npm run gui:incident-live` is intentionally opt-in because it drives the desktop GUI and live runtime approval UI.
- Provider-reported cache hits are still unavailable; current cache events are runtime prefix-reuse observations keyed by DeepSeek zone hashes.
