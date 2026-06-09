# Phase 0 Execution Order

本文件解决的问题：把 Phase 0 从泛泛 task backlog 重排为 scaffold 前真正应该先做的 12 个任务。当前不能 scaffold；scaffold 前必须完成 Phase 0 go/no-go。

修正旧文档的方式：`24_revised_first_30_codex_tasks.md` 仍是 backlog，但本文件是执行顺序。DeepSeek/Qwen native optimization 不重写，只做 consolidation check；compatible provider adapter 可以 spike，但不能影响 native adapters。

## Dependency Summary

Must be serial first:

1. Kernel schema consistency check.
2. PlanApproval vs Permission schema fix.
3. ProviderConfig / ModelAlias schema spike.
4. Native optimization consolidation check.
5. TaskContract schema.

Can run after first five, with limited parallelism:

- Event log replay spike.
- Patch validator spike.
- Command permission classifier spike.
- DeepSeek parser fixture spike.
- Qwen parser/executor fixture spike.
- Compatible provider adapter spike.
- Research CSV profiler spike.

## Task 01: Kernel Schema Consistency Check

- **Goal:** Check Product Kernel schema names/events against latest convergence docs.
- **Why now:** All later spikes depend on schema consistency.
- **Allowed autonomy level:** Single agent, docs-only.
- **Allowed paths:** `docs/agent_architecture_planning/21_product_kernel_v0.md`, `docs/schemas/`.
- **Denied paths:** product source, lock files, secrets.
- **Allowed tools:** read, rg, sed, apply_patch, python text check.
- **Multi-agent allowed:** No.
- **Reviewer required:** Yes if schema changes.
- **Integrator required:** No.
- **Test command:** `rg 'PlanApproval|PermissionRequest|KernelEvent|CompatibleProvider' docs/agent_architecture_planning/21_product_kernel_v0.md docs/schemas || true`
- **Acceptance criteria:** Kernel docs distinguish plan approval from permission and mention compatible provider boundary.
- **Stop condition:** Requires product code or DB migration.
- **Blocks scaffold if:** PlanApproval/Permission remain mixed.
- **Rollback plan:** Revert doc/schema patch.

## Task 02: PlanApproval vs Permission Schema Fix

- **Goal:** Define `PlanApprovalRequest/Decision` separately from `PermissionRequest/Decision`.
- **Why now:** GUI flows and event log need clear governance vs safety boundary.
- **Allowed autonomy level:** Single agent.
- **Allowed paths:** `docs/schemas/`, `docs/agent_architecture_planning/21_product_kernel_v0.md`, `docs/agent_architecture_planning/23_gui_user_flows.md`.
- **Denied paths:** runtime code, permission manager code.
- **Allowed tools:** read, rg, apply_patch, python JSON validation.
- **Multi-agent allowed:** No.
- **Reviewer required:** Yes.
- **Integrator required:** No.
- **Test command:** `rg 'request_type = plan|PlanApprovalRequest|plan.approval' docs/agent_architecture_planning docs/schemas || true`
- **Acceptance criteria:** No `PermissionRequest` with `request_type = plan`; plan approval events/schemas exist.
- **Stop condition:** Unclear event naming impacts kernel.
- **Blocks scaffold if:** Any plan approval still uses permission event.
- **Rollback plan:** Restore previous docs and list open issue.

## Task 03: ProviderConfig / ModelAlias Schema Spike

- **Goal:** Draft `CompatibleProviderConfig` and `ModelAliasMapping` schemas.
- **Why now:** Prevent compatible provider scope from polluting native adapters.
- **Allowed autonomy level:** Docs/schema spike.
- **Allowed paths:** `docs/schemas/provider/`, `docs/agent_architecture_planning/27_model_scope_and_provider_layer.md`.
- **Denied paths:** native adapter docs except references; product code.
- **Allowed tools:** read, rg, apply_patch, python JSON validation.
- **Multi-agent allowed:** No for schema; reviewer allowed.
- **Reviewer required:** Yes.
- **Integrator required:** No.
- **Test command:** `rg 'optimization_level|actual_model_name|display_model_name|model_alias' docs/schemas docs/agent_architecture_planning/27_model_scope_and_provider_layer.md`
- **Acceptance criteria:** compatible provider schema rejects native optimization level.
- **Stop condition:** Requires live provider health check.
- **Blocks scaffold if:** Compatible providers can be configured as native.
- **Rollback plan:** Delete provider schema spike.

## Task 04: Native Optimization Consolidation Check

- **Goal:** Verify DeepSeek/Qwen invariants are preserved and not rewritten generically.
- **Why now:** This is the project’s core product boundary.
- **Allowed autonomy level:** Single agent docs review.
- **Allowed paths:** `docs/agent_architecture_planning/03_*`, `10_*`, `15_*`, `28_*`.
- **Denied paths:** native adapter implementation code.
- **Allowed tools:** read, rg, sed, apply_patch.
- **Multi-agent allowed:** Read-only reviewer only.
- **Reviewer required:** Yes.
- **Integrator required:** Yes if reviewer reports conflict.
- **Test command:** `rg 'reasoning_content|qwen3_coder|prefix-cache|262K|DSML|Qwen3.6-27B' docs/agent_architecture_planning/{03_deepseek_tui_and_docs_analysis.md,10_model_optimization_architecture.md,15_native_deepseek_qwen_modes.md,28_native_optimization_consolidation.md}`
- **Acceptance criteria:** DeepSeek/Qwen invariants appear in authoritative docs with eval gates.
- **Stop condition:** Evidence missing or Qwen source needs network refresh.
- **Blocks scaffold if:** Native invariants are absent or contradicted.
- **Rollback plan:** Restore prior docs and record missing evidence.

## Task 05: TaskContract Schema

- **Goal:** Draft TaskContract schema and examples for docs/spike/implementation tasks.
- **Why now:** Enables bounded autonomy before long Codex tasks or product agent sessions.
- **Allowed autonomy level:** Single agent.
- **Allowed paths:** `docs/schemas/task_contract/`, `docs/agent_architecture_planning/30_bounded_autonomy_task_contract.md`.
- **Denied paths:** runtime scheduler code.
- **Allowed tools:** read, rg, apply_patch, python JSON validation.
- **Multi-agent allowed:** No.
- **Reviewer required:** Yes.
- **Integrator required:** No.
- **Test command:** `rg 'allowed_paths|denied_paths|max_parallel_agents|stop_conditions' docs/schemas docs/agent_architecture_planning/30_bounded_autonomy_task_contract.md`
- **Acceptance criteria:** Contract covers paths/tools/shell/network/package/cloud/retries/agents/tests/artifacts/report.
- **Stop condition:** Contract conflicts with security rules.
- **Blocks scaffold if:** No bounded autonomy schema exists.
- **Rollback plan:** Delete schema draft and keep doc only.

## Task 06: Event Log Replay Spike

- **Goal:** Create replayable JSONL sequence for one coding task.
- **Why now:** Verifies kernel event flow before storage/runtime code.
- **Allowed autonomy level:** Docs/prototype.
- **Allowed paths:** `docs/prototypes/event_log_replay/`.
- **Denied paths:** product code.
- **Allowed tools:** read, rg, apply_patch, python JSONL check.
- **Multi-agent allowed:** Yes, max 2 if one reviewer only.
- **Reviewer required:** Yes.
- **Integrator required:** No.
- **Test command:** `python3 scripts/validate_event_sequence.py docs/prototypes/event_log_replay/coding_task_sequence.jsonl`
- **Acceptance criteria:** Sequence includes plan approval, permission, patch, artifact, eval events.
- **Stop condition:** Needs implementation runtime.
- **Blocks scaffold if:** Event order cannot represent GUI flows.
- **Rollback plan:** Delete prototype sequence.

## Task 07: Patch Validator Spike

- **Goal:** Prototype read-before-write/stale hash/protected path validation.
- **Why now:** Patch safety is release-blocking.
- **Allowed autonomy level:** Isolated spike.
- **Allowed paths:** `scripts/prototype_patch_validator.py`, `eval/fixtures/patch/`.
- **Denied paths:** product runtime.
- **Allowed tools:** read, rg, apply_patch, python script run.
- **Multi-agent allowed:** No.
- **Reviewer required:** Yes.
- **Integrator required:** No.
- **Test command:** `python3 scripts/prototype_patch_validator.py eval/fixtures/patch`
- **Acceptance criteria:** stale/protected/ambiguous patches fail safely.
- **Stop condition:** Needs product Patch Manager.
- **Blocks scaffold if:** Stale base hash can apply.
- **Rollback plan:** Delete script/fixtures.

## Task 08: Command Permission Classifier Spike

- **Goal:** Prototype conservative command classification.
- **Why now:** Shell command injection/destructive commands are P0 threats.
- **Allowed autonomy level:** Isolated spike.
- **Allowed paths:** `scripts/prototype_command_classifier.py`, `eval/fixtures/shell/permission_cases.json`.
- **Denied paths:** shell executor product code.
- **Allowed tools:** read, rg, apply_patch, python script run.
- **Multi-agent allowed:** No.
- **Reviewer required:** Yes.
- **Integrator required:** No.
- **Test command:** `python3 scripts/prototype_command_classifier.py eval/fixtures/shell/permission_cases.json`
- **Acceptance criteria:** destructive/network/install commands deny or require approval.
- **Stop condition:** Requires executing commands.
- **Blocks scaffold if:** denied command can be classified allow.
- **Rollback plan:** Delete spike files.

## Task 09: DeepSeek Parser Fixture Spike

- **Goal:** Create DeepSeek XML/JSON/native tool-call fixture set.
- **Why now:** DeepSeek native promotion depends on parser gates.
- **Allowed autonomy level:** Fixture spike.
- **Allowed paths:** `eval/fixtures/deepseek/`, `docs/prototypes/deepseek_parser_eval.md`.
- **Denied paths:** native adapter implementation.
- **Allowed tools:** read, rg, apply_patch, python JSON validation.
- **Multi-agent allowed:** Yes, max 2 if disjoint fixture sections.
- **Reviewer required:** Yes.
- **Integrator required:** Yes.
- **Test command:** `python3 -m json.tool eval/fixtures/deepseek/parser_golden.json`
- **Acceptance criteria:** wrong-tool and low-confidence repair cases are deny/retry, not execute.
- **Stop condition:** Needs live DeepSeek output or network.
- **Blocks scaffold if:** No parser promotion fixture exists.
- **Rollback plan:** Delete fixture file.

## Task 10: Qwen Parser/Executor Fixture Spike

- **Goal:** Create Qwen3.6 parser, template, thinking/non-thinking, hallucinated-file executor fixtures.
- **Why now:** Qwen native target needs evidence stronger than generic provider compatibility.
- **Allowed autonomy level:** Fixture spike.
- **Allowed paths:** `eval/fixtures/qwen/`, `docs/prototypes/qwen_parser_eval.md`.
- **Denied paths:** Qwen native adapter implementation.
- **Allowed tools:** read, rg, apply_patch, python JSON validation.
- **Multi-agent allowed:** Yes, max 2 if one handles parser and one executor fixtures.
- **Reviewer required:** Yes.
- **Integrator required:** Yes.
- **Test command:** `python3 -m json.tool eval/fixtures/qwen/parser_golden.json`
- **Acceptance criteria:** fixtures distinguish qwen3 parser/template capability from generic OpenAI transport.
- **Stop condition:** Need current Qwen docs/network; mark unknowns instead.
- **Blocks scaffold if:** Qwen native mode lacks parser/executor gates.
- **Rollback plan:** Delete fixture file.

## Task 11: Compatible Provider Adapter Spike

- **Goal:** Prototype request/response transform spec for OpenAI-compatible and custom provider without native optimization.
- **Why now:** Other providers need clean compatible-only path.
- **Allowed autonomy level:** Docs/prototype.
- **Allowed paths:** `docs/prototypes/provider_adapter/`, `docs/schemas/provider/`.
- **Denied paths:** DeepSeek/Qwen native adapters.
- **Allowed tools:** read, rg, apply_patch, python JSON validation.
- **Multi-agent allowed:** No for first schema.
- **Reviewer required:** Yes.
- **Integrator required:** No.
- **Test command:** `rg 'request_transform|response_transform|optimization_level' docs/prototypes/provider_adapter docs/schemas/provider`
- **Acceptance criteria:** compatible adapter cannot set native optimization and cannot override native parser/context.
- **Stop condition:** Needs live provider health check.
- **Blocks scaffold if:** compatible path is undefined or pollutes native adapters.
- **Rollback plan:** Delete prototype.

## Task 12: Research CSV Profiler Spike

- **Goal:** Prototype smallest CSV data-quality profile and PII classification.
- **Why now:** Research Coworker must stay first-class and privacy-aware.
- **Allowed autonomy level:** Isolated spike.
- **Allowed paths:** `scripts/prototype_csv_profiler.py`, `eval/fixtures/research/csv-quality-small/`.
- **Denied paths:** production research worker.
- **Allowed tools:** read, rg, apply_patch, python script run.
- **Multi-agent allowed:** Yes, only if fixture and script are disjoint then integrated.
- **Reviewer required:** Yes.
- **Integrator required:** Yes if multi-agent.
- **Test command:** `python3 scripts/prototype_csv_profiler.py eval/fixtures/research/csv-quality-small/input.csv`
- **Acceptance criteria:** detects missing values, duplicates, and sensitive columns; emits artifact-like JSON.
- **Stop condition:** Requires package install or network.
- **Blocks scaffold if:** Research Worker privacy/profile contract cannot be represented.
- **Rollback plan:** Delete script/fixture.

## Parallelization Rules

- Tasks 01-05 are serial.
- Tasks 06-12 can run after 01-05.
- Tasks 09 and 10 may run in parallel.
- Tasks 07 and 08 may run in parallel if write paths remain separate.
- Task 11 must not modify native adapter docs.

## Phase 0 Go/No-Go

Go to scaffold only when:

- all 12 tasks are complete or explicitly deferred with risk accepted;
- release-blocking security rules have executable checks or fixture specs;
- compatible provider cannot be marked native;
- DeepSeek/Qwen native invariants are preserved;
- PlanApproval/Permission split is complete;
- TaskContract exists.

Current status: **No-Go for scaffold. Go for Phase 0 spikes after this convergence pass.**

