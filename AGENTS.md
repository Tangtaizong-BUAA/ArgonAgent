# AGENTS.md

## Product Direction

This project is a DeepSeek/Qwen-first local AI agent workbench:

- Claude Code-like local coding runtime;
- Codex GUI-like command center;
- Research Coworker for data, experiments, reports, and literature workflows;
- local-first architecture;
- native optimization only for DeepSeek and Qwen/Qwen3.6-27B.

## Current Work State

The current repo baseline is past scaffold work. Treat the project as an
active runtime-hardening implementation, not as an architecture-only planning
exercise.

Current status snapshots:

- Phase 0 and Phase 1 runtime/kernel gates have passed locally.
- The current engineering phase is ClaudeCode-grade reliability hardening for
  long-running DeepSeek/Qwen native agent work.
- P2-B conversation-history/OpenAI JSON projection, P2-C transient provider
  retry, and P3 AgentKernel authority-boundary work are locally implemented and
  verified.
- P4 remains partial: long-session replay/compaction tests, subagent event
  merge/isolation, desktop stream normalization, transcript performance, and
  live-provider gated smoke coverage still need work.

Authoritative current-state documents:

- `docs/implementation/implementation_status.md`
- `docs/runtime/p3_p4_completion_status_2026_05_19.md`
- `docs/implementation/agent_kernel_tool_contract_long_task_todos.md`
- `docs/agent_architecture_planning/39_deepseek_native_agent_kernel_architecture_and_plan.md`

Implementation should continue from the latest accepted plan/backlog and the
current code, not restart from old scaffold assumptions. In particular:

- root `desktop/` is the real Tauri GUI path; `apps/desktop/` is not the
  product GUI target unless a newer plan explicitly says otherwise.
- `RuntimeFacade -> AgentKernel -> NativeProfile(DeepSeek/Qwen)` is the target
  runtime ownership chain; do not add competing loop-policy owners.
- Tool, permission, event, compaction, conversation-history, and GUI surfaces
  must remain replayable and structured.
- DeepSeek/Qwen native behavior remains first-class kernel/profile behavior,
  not a generic compatible-provider adapter tweak.

## Model Scope

Native optimized models:

- DeepSeek
- Qwen / Qwen3.6-27B

Compatible-only providers:

- Claude
- OpenAI / Codex / GPT
- GLM
- local models
- OpenAI-compatible APIs
- Anthropic-compatible APIs
- custom providers

Rules:

- Compatible providers can be manual options, baselines, or explicitly approved fallbacks.
- Compatible providers cannot be marked native.
- Compatible providers cannot override DeepSeek/Qwen native prompts, parsers, context policy, tool policy, or eval gates.
- Use `ProviderConfig` and `ModelAliasMapping` for compatible providers.
- Do not genericize DeepSeek/Qwen optimization into ordinary provider configs.

## Native Optimization Preservation

Existing DeepSeek/Qwen optimization docs are authoritative:

- `docs/agent_architecture_planning/03_deepseek_tui_and_docs_analysis.md`
- `docs/agent_architecture_planning/10_model_optimization_architecture.md`
- `docs/agent_architecture_planning/15_native_deepseek_qwen_modes.md`
- `docs/agent_architecture_planning/20_eval_suite_v0.md`
- `docs/agent_architecture_planning/28_native_optimization_consolidation.md`

Any change to native optimization must state:

- reason for change;
- affected profile/rule;
- affected eval case;
- promotion condition;
- rollback condition.

## Multi-Agent Policy

Default = Single Agent + Reviewer.

Allowed multi-agent scenarios:

- independent source research;
- isolated spike prototypes;
- eval fixture generation in separate directories;
- adversarial read-only review;
- implementation shards after interfaces freeze.

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

## Autonomous Task Policy

For this repository, user requests to build, fix, harden, align with
ClaudeCode/OpenCode, continue from a breakpoint, complete a plan, or finish
remaining implementation work are long-horizon implementation tasks by default.
Do not reinterpret those requests as "one assistant response equals one small
step." Continue through the unblocked dependency chain until the task is
complete, a TaskContract boundary is reached, or a real stop condition applies.

Simple direct questions, read-only reviews, status checks, and explicit
"explain only / do not edit" requests remain narrow. Answer them directly and
do not silently convert them into implementation runs.

Every long task requires a TaskContract with:

- goal and scope;
- allowed and denied paths;
- allowed and denied tools;
- shell, network, package-install, and cloud-model policies;
- max duration, retries, and parallel agents;
- required tests and artifacts;
- stop conditions;
- reviewer/integrator requirements;
- final report format.

Boundary rule:

- inside contract: execute autonomously;
- outside contract: stop and report.

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

Default execution style for this project is long-horizon autonomous work, not
three-step micro-bursts. A final response should represent a completed
coherent objective, a verified checkpoint requested by the user, or a true
boundary/blocker, not an arbitrary pause after a few actions.

When the user asks to build, fix, harden, align with ClaudeCode/OpenCode, or
complete a subsystem, the agent should:

- create or maintain a multi-step todo list large enough to cover the real
  dependency chain;
- work through the todo list continuously within the current TaskContract;
- batch related edits before running verification;
- run the strongest practical local checks after each coherent slice;
- continue into the next unblocked slice without asking for "continue";
- preserve runtime/GUI boundaries while making bottom-up implementation
  progress;
- stop only at explicit safety, permission, architecture, dependency, or test
  blockers.

No small-step contract:

- do not stop after one file, one test, or one visible improvement if the
  requested plan still has unblocked work;
- do not ask the user to say "continue" between ordinary implementation slices;
- do not turn passing a smoke test into a final answer when stronger required
  checks remain;
- do not replace execution with a new plan unless the user asked for planning
  or the TaskContract requires a revised plan before edits;
- progress updates are status updates, not completion reports.

Intent-classification guard:

- classify the current turn from the latest user request and active plan state,
  not from incidental words inside `AGENTS.md`, runtime context, summaries, or
  previous instructions;
- a small question should not become a long task merely because the project
  context contains phrases such as "long-horizon", "complete the plan", or
  "continue implementation";
- a real long implementation request should not be artificially shortened just
  because an assistant message budget or prior habit suggests doing only one
  micro-step.

Do not artificially stop after three actions, three files, or three tasks. A
final answer should be sent only when one of these is true:

- all unblocked items in the requested plan/backlog are genuinely completed and
  verified;
- a TaskContract boundary is reached;
- a denied tool/path/network/package/security action is required;
- tests expose a blocker that cannot be fixed without changing scope;
- the user explicitly asks for status, review, or pause.

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

For long implementation sessions, prefer this cadence:

1. inspect enough code to avoid guessing;
2. update a todo/checklist;
3. implement the next coherent runtime/tool/context/TUI slice;
4. run focused tests;
5. repair failures;
6. broaden tests;
7. continue to the next unblocked slice.

Intermediate user-facing updates should be concise and periodic, but they must
not replace doing the work. The agent should not end the turn merely because one
small phase passed.

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

Final responses must include:

- changed files;
- concise summary;
- tests/checks run;
- risks;
- unresolved questions;
- next recommended task.
