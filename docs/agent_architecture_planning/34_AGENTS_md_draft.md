# 34 AGENTS.md Draft

本文件是项目根目录 `AGENTS.md` 草案。不要自动覆盖根目录 `AGENTS.md`；需要用户明确批准后再替换。

## Product Direction

This project is a DeepSeek/Qwen-first local AI agent workbench:

- Claude Code-like local coding runtime;
- Codex GUI-like command center;
- Research Coworker for data, experiments, reports, and literature workflows;
- local-first architecture;
- native optimization only for DeepSeek and Qwen/Qwen3.6-27B.

## Model Scope

### Native Models

- DeepSeek = native optimized.
- Qwen / Qwen3.6-27B = native optimized.

Native models have dedicated:

- model profile;
- system prompt and role prompts;
- tool-use strategy;
- context strategy;
- parser;
- error recovery;
- long-task continuation;
- eval suite;
- failure memory;
- profile tuning.

### Compatible Providers

Claude, OpenAI/Codex/GPT, GLM, local models, OpenAI-compatible APIs, Anthropic-compatible APIs, and custom providers are compatible-only.

Rules:

- Compatible providers can be manual options, baselines, or explicitly approved fallbacks.
- Compatible providers cannot be marked native.
- Compatible providers cannot override DeepSeek/Qwen native prompts, parsers, context policy, tool policy, or eval gates.
- Use `ProviderConfig` and `ModelAliasMapping` with `base_url`, `actual_model_name`, `display_model_name`, `model_alias`, `request_transform`, and `response_transform`.
- Do not genericize DeepSeek/Qwen optimization into ordinary provider configs.

## Native Optimization Preservation

Existing DeepSeek/Qwen optimization docs are authoritative:

- `docs/agent_architecture_planning/03_deepseek_tui_and_docs_analysis.md`
- `docs/agent_architecture_planning/10_model_optimization_architecture.md`
- `docs/agent_architecture_planning/15_native_deepseek_qwen_modes.md`
- `docs/agent_architecture_planning/20_eval_suite_v0.md`
- `docs/agent_architecture_planning/28_native_optimization_consolidation.md`

Do not rewrite native profiles as generic provider profiles.

Changes require:

- reason for change;
- affected profile/rule;
- affected eval case;
- promotion condition;
- rollback condition.

## Context Budget and Scaffold Policy

ClaudeCode is the primary reference for mature agent scaffold engineering. Learn its lifecycle discipline, prompt hierarchy, tool schema stability, permission boundaries, patch/test/review loop, memory/compaction pattern, and error recovery. Do not copy it as one giant model-agnostic prompt.

Rules:

- Runtime scaffold must stay strong for every native session: event log, state machine, TaskContract, permission manager, patch manager, read-before-write, base hash, stale-file detection, command classifier, tool dispatcher, artifact store, reviewer loop, and eval harness.
- Do not weaken runtime scaffold to save tokens.
- Prompt scaffold is model-specific and role-specific.
- DeepSeek may use S3 Full Scaffold: closest to ClaudeCode full lifecycle, stable prefix/tool catalog, reasoning replay, late compaction, and larger context.
- Qwen3.6-27B uses S1 Fast or S2 Guarded Scaffold: ClaudeCode-lite prompt, narrow tools, small patch loop, runtime validators, tests, and reviewer gates.
- Compatible providers use S0 Minimal Scaffold only.
- Qwen must not receive DeepSeek S3 full scaffold by default.
- Full prompt scaffold changes require eval evidence before promotion.
- Use `docs/agent_architecture_planning/36_context_budget_and_scaffold_policy.md` and `crates/runtime/src/context_budget.rs` as the current source of truth.

## Multi-Agent Policy

Default = Single Agent + Reviewer.

Allowed multi-agent scenarios:

- independent source research;
- isolated spike prototypes;
- eval fixture generation in separate directories;
- adversarial read-only review;
- implementation shards after interfaces freeze;
- non-overlapping docs edits with clear ownership.

Banned multi-agent scenarios:

- Product Kernel design;
- Event schema design;
- Database schema design;
- Permission Manager core;
- Patch Manager core;
- Model Router core;
- DeepSeek/Qwen native adapter core strategy;
- Security model;
- ADR decisions;
- root architecture contract;
- AGENTS.md core rules.

Integrator rules:

- owns final merge;
- deduplicates outputs;
- resolves conflicts;
- rejects changes that weaken native invariants or security gates.

Reviewer rules:

- read-only by default;
- reports findings with file/section references;
- does not silently edit core architecture decisions.

Worktree rules:

- product code parallelism requires worktree isolation;
- spikes may use isolated directories;
- no two agents may edit the same file.

Max parallelism:

- current convergence: 1 main + optional reviewer;
- Phase 0 spikes: max 4, isolated;
- implementation after interface freeze: max 3 implementation agents + reviewer;
- kernel/security/native adapter work: single agent only.

## Autonomous Task Policy

Every long task requires a TaskContract with:

- goal;
- scope;
- allowed_paths;
- denied_paths;
- allowed_tools;
- denied_tools;
- shell_policy;
- network_policy;
- package_install_policy;
- cloud_model_policy;
- max_duration;
- max_retries;
- max_parallel_agents;
- required_tests;
- required_artifacts;
- stop_conditions;
- reviewer_required;
- integrator_required;
- final_report_format.

Default allowed:

- read project files;
- modify `docs/`;
- modify `spikes/`;
- modify `tests/fixtures/`;
- run non-destructive check commands;
- generate reports;
- generate eval fixtures;
- generate small spike prototypes.

Default denied:

- read `.env`;
- read private keys or SSH keys;
- upload files;
- network access;
- install dependencies;
- delete files;
- force push;
- modify git history;
- modify lock files;
- modify Product Kernel/Event schema/Permission Manager/Patch Manager/Model Router core/Security model/native adapter core strategy without explicit authorization.

Escalate/stop when:

- task needs denied path/tool;
- package install or network is required;
- secret/protected path appears;
- retry budget is exceeded;
- schema/security/native adapter changes are required but not authorized.

Final report is required for every long task.

When the user explicitly asks for long-running implementation, "continue from
the breakpoint", "complete the whole plan", "finish all remaining plan items",
or equivalent wording, infer an implementation TaskContract from the latest
accepted plan/backlog in the repo instead of asking the user to restate it.

Default inferred implementation TaskContract:

- goal: complete every unblocked item in the active plan/backlog;
- scope: current repository implementation and harness hardening;
- allowed paths: project source, tests, fixtures, docs, scripts, and local
  runtime/TUI code needed by the active plan;
- denied paths: secrets, private keys, SSH keys, dependency lockfiles unless
  explicitly part of the task, git history, and unrelated user files;
- allowed tools: read/search/edit/patch, non-destructive local tests/checks,
  git status/diff/log/add/commit for checkpoints;
- denied tools: destructive shell, secret reads, package installs, network
  upload, force push, git history rewrite;
- required tests: focused tests for each coherent slice, then broader harness
  checks before final report;
- stop conditions: hard-deny boundary, missing dependency requiring install,
  network requirement not already authorized, repeated unresolved test failure,
  architecture contradiction, or task exceeding the active plan.

## Long-Horizon Work Policy

Default execution style for this project is **long-horizon autonomous work**.
Agents should not treat three steps, three files, or one small smoke test as a
natural stopping point when the user asked for a subsystem, hardening pass, or
ClaudeCode/OpenCode parity.

Rules:

- Create or maintain a todo list that reflects the real dependency chain.
- Execute continuously inside the approved TaskContract.
- Batch related implementation work into coherent slices.
- Verify each coherent slice with focused tests, then broaden tests before the
  final report.
- Continue into the next unblocked slice without asking the user to say
  "continue".
- Preserve the runtime boundary: `RuntimeFacade -> AgentEvent -> TUI/GUI`.
- Stop only for safety/permission boundaries, denied paths/tools, package or
  network approval needs, architecture-contract conflicts, or unresolved test
  blockers.

## Plan Completion Mandate

When the user says to complete "all plan items", "all remaining work", or
"全部完成并落实", the active plan is treated as the execution contract. The
agent must not stop after implementing only a representative subset.

Required behavior:

- keep a visible todo/checklist internally and update it as work progresses;
- execute plan items in dependency order;
- after a slice passes focused tests, immediately continue to the next
  unblocked slice;
- if a slice is blocked, record the blocker and continue other independent
  unblocked slices;
- run final broad checks only after all unblocked slices are done;
- create git checkpoints after coherent verified slices when git is available;
- final report only after all unblocked work is complete or after a true stop
  condition is reached.

Forbidden behavior:

- do not stop merely because a smoke test passed;
- do not ask the user to say "continue" between plan items;
- do not replace implementation with another architecture document unless the
  active plan explicitly asks for documentation;
- do not silently skip failing or complex plan items; mark them blocked with
  evidence and continue other safe work.

## Engineering Completion Gate

A long implementation task is not complete until the requested capability is
usable at engineering quality, not merely scaffolded or smoke-tested.

Completion requires all applicable items below:

- runtime path is implemented, not only documented;
- TUI/CLI behavior is wired through `RuntimeFacade -> AgentEvent`;
- tool, permission, patch, session, context, and model paths have coherent
  error handling;
- DeepSeek and Qwen native-specific behavior stays separated from compatible
  providers;
- unsupported or gated capabilities fail with structured recoverable events,
  not crashes or silent no-ops;
- focused tests cover the changed slice;
- broader harness checks pass or have documented blockers;
- user-facing behavior can be reproduced with a command or fixture;
- event logs remain replayable and GUI-consumable;
- risks, blocked items, and remaining gaps are explicitly reported.

If any requested plan item remains unimplemented, partially wired, or failing
without a hard stop condition, the agent must continue working. Do not present a
partial implementation as final product completion.

Recommended cadence:

1. inspect source and current status;
2. refresh todo/checklist;
3. implement the next runtime/tool/context/TUI slice;
4. run focused tests;
5. repair failures;
6. run broader tests;
7. proceed to the next unblocked slice.

Intermediate updates should stay concise and periodic. They must not become a
replacement for sustained execution.

## File Modification Rules

Allowed by default:

- docs;
- spikes;
- eval fixtures;
- small prototype scripts when TaskContract permits.

Require explicit authorization:

- kernel schema;
- event schema;
- database schema;
- permission manager;
- patch manager;
- model router core;
- DeepSeek/Qwen native adapter core strategy;
- security model;
- root `AGENTS.md`.

Never:

- read secrets;
- delete files without explicit instruction;
- install dependencies without approval;
- modify git history;
- bypass PatchProposal for product code writes.

## Testing Rules

- Every implementation task needs tests.
- Every model optimization needs eval evidence.
- Every native profile promotion needs eval gate.
- DeepSeek parser wrong-tool execution blocks promotion.
- Qwen parser wrong-tool execution blocks promotion.
- Compatible providers cannot bypass eval when used as baselines.
- Compatible providers cannot enter native eval promotion.

## Output Rules

Final response must include:

- changed files;
- concise summary;
- tests/checks run;
- risks;
- unresolved questions;
- next recommended task.

For review tasks, findings come first and must cite file/section.

## Open Questions

- Should root `AGENTS.md` include only concise policy and link to docs, or embed this full draft?
- Should compatible providers be hidden behind advanced settings in the first GUI?
