# 12 First 30 Codex Tasks

These tasks intentionally start from infrastructure and thin vertical slices. They are small enough for Codex-style execution but aligned with the full architecture.

## 1. Create Monorepo Skeleton
- Goal: Create `apps/desktop`, `crates/runtime`, `crates/cli`, `crates/storage`, `python/research_worker`.
- Why: Establish boundaries early.
- Input context: `08_target_product_architecture.md`.
- Files: workspace manifests, README, `docs/engineering/reference_use_policy.md`.
- Plan: create folders, Rust workspace, package.json, Python pyproject, basic docs, and clean-room/reference-use policy before implementation code.
- Acceptance: all empty packages build or print help.
- Test command: `cargo check --workspace`.
- Risk: premature structure churn; unclear reference-code reuse boundaries.
- Rollback: remove created skeleton.
- Approval: no.

## 2. Add Shared Type Schema Draft
- Goal: Encode core interfaces as JSON schema or Rust/TS types.
- Why: GUI/runtime agreement.
- Input: section E in `08_target_product_architecture.md`.
- Files: `crates/runtime/src/types.rs`, `apps/desktop/src/types.ts`.
- Plan: define IDs/enums/events, add serialization tests.
- Acceptance: generated sample serializes/deserializes.
- Test: `cargo test -p runtime types`.
- Risk: schema overfit.
- Rollback: revert files.
- Approval: no.

## 3. SQLite Migration Runner
- Goal: Add local DB and migrations.
- Why: Event persistence.
- Input: storage schema in `08`.
- Files: `crates/storage/*`, migrations.
- Plan: choose sqlx/rusqlite, create tables, migration test.
- Acceptance: temp DB migrates cleanly.
- Test: `cargo test -p storage`.
- Risk: library choice.
- Rollback: delete crate.
- Approval: no.

## 4. Agent State Machine
- Goal: Implement state transitions.
- Why: Runtime correctness.
- Input: state table in `08`.
- Files: `crates/runtime/src/state.rs`.
- Plan: enum, transition guard, error types, tests.
- Acceptance: invalid transitions rejected.
- Test: `cargo test -p runtime state`.
- Risk: missing states.
- Rollback: revert state module.
- Approval: no.

## 5. Event Log API
- Goal: Append/list session events.
- Why: GUI timeline.
- Input: `AgentEvent`.
- Files: `crates/runtime/src/events.rs`, `crates/storage`.
- Plan: append API, DB persistence, replay order test.
- Acceptance: events survive restart.
- Test: `cargo test -p runtime event`.
- Risk: event schema churn.
- Rollback: migration down/revert.
- Approval: no.

## 6. Runtime Mock Model Provider
- Goal: Deterministic model fixture provider.
- Why: Test without API cost.
- Input: ClawCode mock parity idea.
- Files: `crates/runtime/src/model/mock.rs`.
- Plan: scriptable responses with tool calls.
- Acceptance: fixture emits text/tool events.
- Test: `cargo test -p runtime mock_model`.
- Risk: too simple.
- Rollback: revert.
- Approval: no.

## 7. ToolSpec Trait
- Goal: Define typed tool interface.
- Why: Foundation for all tools.
- Input: ClaudeCode `Tool.ts`, OpenCode `tool.ts`.
- Files: `crates/runtime/src/tool.rs`.
- Plan: trait, schema metadata, permission metadata.
- Acceptance: mock tool implements trait.
- Test: `cargo test -p runtime tool`.
- Risk: trait too broad.
- Rollback: revert.
- Approval: no.

## 8. File Read Tool
- Goal: Implement safe line-window file read.
- Why: First real tool.
- Input: ClaudeCode FileRead, Claw file_ops.
- Files: `crates/runtime/src/tools/read_file.rs`.
- Plan: path normalization, max size, binary check, line window.
- Acceptance: reads fixture, rejects binary/oversize.
- Test: `cargo test -p runtime read_file`.
- Risk: path boundary.
- Rollback: revert tool.
- Approval: no.

## 9. Ripgrep Search Tool
- Goal: Add `rg`-backed search.
- Why: Repo context retrieval.
- Input: agent requirements.
- Files: `crates/runtime/src/tools/search.rs`.
- Plan: command wrapper, limits, structured matches.
- Acceptance: finds fixture matches.
- Test: `cargo test -p runtime search`.
- Risk: rg unavailable.
- Rollback: revert or fallback.
- Approval: no.

## 10. Permission Request Model
- Goal: Implement allow/deny/ask request records.
- Why: safe tools.
- Input: ClaudeCode/OpenCode permissions.
- Files: `crates/runtime/src/permission.rs`.
- Plan: policy enum, request, decision, tests.
- Acceptance: denied tool returns denial event.
- Test: `cargo test -p runtime permission`.
- Risk: simplistic matching.
- Rollback: revert.
- Approval: no.

## 11. Shell Tool Stub With Approval
- Goal: Add shell command tool behind permission.
- Why: build/test execution.
- Input: BashTool, OpenCode shell.
- Files: `crates/runtime/src/tools/shell.rs`.
- Plan: timeout, cwd, output capture, approval gate, no sandbox yet.
- Acceptance: approved echo runs; denied does not.
- Test: `cargo test -p runtime shell`.
- Risk: unsafe defaults.
- Rollback: disable tool.
- Approval: human approval for running arbitrary commands during testing if needed.

## 12. PatchProposal Data Model
- Goal: Store proposed diffs without applying.
- Why: GUI review.
- Input: Patch Manager design.
- Files: `crates/runtime/src/patch.rs`, DB migration.
- Plan: unified diff parser/store, statuses.
- Acceptance: patch proposal persists and renders.
- Test: `cargo test -p runtime patch`.
- Risk: parser edge cases.
- Rollback: revert migration.
- Approval: no.

## 13. Apply Patch Tool
- Goal: Apply approved patches to workspace.
- Why: first edit path.
- Input: ClaudeCode/OpenCode patch patterns.
- Files: patch manager.
- Plan: validate approval, apply, conflict detection, backup.
- Acceptance: fixture patch applies/reverts.
- Test: `cargo test -p runtime apply_patch`.
- Risk: file corruption.
- Rollback: backup restore.
- Approval: yes for non-test workspace writes.

## 14. Runtime Loop With Mock Tool Calls
- Goal: Connect model loop, tool dispatcher, events.
- Why: first vertical slice.
- Input: `ConversationRuntime` patterns.
- Files: `crates/runtime/src/session.rs`.
- Plan: user prompt -> mock model -> tool call -> result -> completion.
- Acceptance: fixture session completes.
- Test: `cargo test -p runtime loop`.
- Risk: loop recursion bugs.
- Rollback: revert session loop.
- Approval: no.

## 15. Local Runtime API
- Goal: Expose create task/session and event stream.
- Why: GUI/CLI clients.
- Files: `crates/runtime-server` or `crates/runtime/src/api.rs`.
- Plan: local HTTP/WebSocket or Tauri IPC abstraction.
- Acceptance: client can subscribe to events.
- Test: API integration test.
- Risk: choosing API too early.
- Rollback: keep in-process API.
- Approval: no.

## 16. CLI `researchcode run`
- Goal: Run a task from terminal.
- Why: developer dogfood.
- Files: `crates/cli`.
- Plan: parse args, call runtime, print events.
- Acceptance: mock task runs.
- Test: `cargo run -p cli -- run "hello"`.
- Risk: UX churn.
- Rollback: revert CLI command.
- Approval: no.

## 17. Tauri App Bootstrap
- Goal: Launch desktop shell.
- Why: GUI command center base.
- Files: `apps/desktop`.
- Plan: Tauri init, React routes, runtime ping.
- Acceptance: window shows runtime health.
- Test: frontend build + Tauri dev.
- Risk: platform setup.
- Rollback: revert app skeleton.
- Approval: no.

## 18. Project List UI
- Goal: Add/manage project roots.
- Why: GUI starts from projects.
- Files: desktop UI + storage API.
- Plan: list, add, remove, recent.
- Acceptance: project persists in DB.
- Test: UI/component test.
- Risk: path permission.
- Rollback: revert route.
- Approval: no.

## 19. Task Board UI
- Goal: Create and show tasks.
- Why: command center.
- Files: desktop UI/runtime API.
- Plan: columns, create modal, status updates.
- Acceptance: task creates session.
- Test: Playwright smoke.
- Risk: UI without backend maturity.
- Rollback: hide route.
- Approval: no.

## 20. Session Timeline UI
- Goal: Render event stream.
- Why: user observability.
- Files: desktop session view.
- Plan: event cards for model/tool/permission/patch.
- Acceptance: mock session renders.
- Test: component snapshots.
- Risk: event schema churn.
- Rollback: generic event renderer.
- Approval: no.

## 21. Permission Approval UI
- Goal: Approve/deny pending requests.
- Why: safety.
- Files: GUI panel + runtime decision endpoint.
- Plan: list pending, detail, decision.
- Acceptance: denied shell stays denied; approved runs.
- Test: integration.
- Risk: race conditions.
- Rollback: CLI-only fallback.
- Approval: no.

## 22. Diff Review UI
- Goal: Render patch proposals.
- Why: trust.
- Files: GUI diff panel.
- Plan: file list, unified diff, approve/reject.
- Acceptance: approved patch applies.
- Test: UI integration.
- Risk: large diffs.
- Rollback: open raw diff artifact.
- Approval: no.

## 23. ModelProfile Registry
- Goal: Store built-in profiles.
- Why: model routing.
- Files: `crates/runtime/src/model/profile.rs`.
- Plan: DeepSeek V4 and Qwen3.6-27B native profiles; ClaudeCode/OpenCode/Codex reference metadata for scaffold/eval notes only.
- Acceptance: profiles API lists native DeepSeek/Qwen profiles and marks all reference systems as non-routable.
- Test: profile validation.
- Risk: stale model names.
- Rollback: mark experimental.
- Approval: no.

## 24. Model Router v1
- Goal: Route planner/executor/reviewer roles.
- Why: native DeepSeek/Qwen mode switching.
- Files: model router.
- Plan: rule-based classifier, selected-family mode switch, same-family fallback list, explicit cross-family fallback block.
- Acceptance: coding task produces a DeepSeek route in DeepSeek mode and a Qwen3.6-27B route in Qwen mode.
- Test: routing tests.
- Risk: too manual.
- Rollback: default single model.
- Approval: no.

## 25. Native Parser Fixtures
- Goal: Test DeepSeek and Qwen3.6-27B reasoning/tool-call handling.
- Why: real native model optimization.
- Files: `crates/runtime/src/model/deepseek.rs`, `crates/runtime/src/model/qwen.rs`, fixtures.
- Plan: parse DeepSeek native tool calls/DSML fallback/arg repair; parse Qwen3.6 reasoning output and Qwen tool-call parser fixtures.
- Acceptance: DeepSeek and Qwen fixtures pass, repairs are logged, and unsupported generic tool formats are rejected.
- Test: `cargo test -p runtime deepseek` and `cargo test -p runtime qwen`.
- Risk: API drift.
- Rollback: disable fallback parser.
- Approval: no.

## 26. ContextBundle Builder
- Goal: Build model context slots.
- Why: reliable prompts.
- Files: context manager.
- Plan: task, plan, repo map, snippets, memory, tool outputs.
- Acceptance: token estimate and ordering deterministic.
- Test: snapshot test.
- Risk: token estimation rough.
- Rollback: simpler bundle.
- Approval: no.

## 27. Python Research Worker Skeleton
- Goal: Start sidecar and run health command.
- Why: research module base.
- Files: `python/research_worker`.
- Plan: FastAPI or stdio RPC, health, environment info.
- Acceptance: runtime can call health.
- Test: Python pytest + Rust integration.
- Risk: packaging.
- Rollback: CLI subprocess only.
- Approval: no.

## 28. Data Profiler v1
- Goal: Profile CSV/Excel/JSON/Parquet.
- Why: first research feature.
- Files: Python worker + runtime tool.
- Plan: sample, schema, quality stats, artifact.
- Acceptance: fixture profiles match expected.
- Test: `pytest python/research_worker`.
- Risk: dependency size.
- Rollback: CSV-only fallback.
- Approval: no.

## 29. Eval Harness v1
- Goal: Local fixture runner.
- Why: prove quality.
- Files: `crates/eval` or runtime eval module.
- Plan: coding tool-loop fixture, metrics JSON.
- Acceptance: mock eval reports pass/fail/tool errors.
- Test: `cargo test -p eval`.
- Risk: artificial eval.
- Rollback: keep as dev tool.
- Approval: no.

## 30. End-to-End Demo Scenario
- Goal: Run mock coding task through GUI with approval and diff.
- Why: validate architecture.
- Files: tests/demo fixtures, docs.
- Plan: create project fixture, run task, approve patch, verify file.
- Acceptance: one command/test drives the full local loop.
- Test: `cargo test --workspace` plus Playwright smoke.
- Risk: flaky UI.
- Rollback: CLI-only demo.
- Approval: no.
