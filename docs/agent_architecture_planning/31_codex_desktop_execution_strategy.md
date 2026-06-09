# 31 Codex Desktop Execution Strategy

本文件解决的问题：说明如何利用 Codex Desktop、多线程和长任务能力执行本项目，同时避免多 agent 破坏核心一致性。

修正旧文档的方式：把“充分利用 Codex 长任务”收敛为阶段化执行策略；当前修整阶段不启用 implementation agents。

## 1. Current Convergence Stage

**Decision:** 当前阶段使用 **单一主 agent**，最多一个 reviewer。

**Rule:** 不开 implementation agents；不并行修改核心文档；不 scaffold；不写产品代码。

**Rationale:** 当前任务影响模型范围、ADR、Product Kernel、PlanApproval/Permission、TaskContract、多智能体规则。这些是 root architecture contracts，必须保持一个主线。

**Implementation Impact:** 当前 Codex Desktop 执行只允许 docs/security/implementation 文档修订和一致性检查。

**Eval Impact:** 当前阶段只准备 eval/spike 顺序，不运行模型 eval、不推广 native profile。

**Go/No-Go Impact:** 当前收敛完成后可以进入 Phase 0 spikes，但仍不能 scaffold。

## 2. Current Stage Parallelism

| Task type | Parallel? | Reason |
|---|---|---|
| 架构收敛文档 | No | 全局一致性高风险 |
| DeepSeek/Qwen consolidation review | No for edits; reviewer allowed | 保留不变量，不重写 |
| compatible provider schema spike | Later yes | 独立 docs/prototypes 可并行 |
| threat control matrix | No for first draft | 安全规则要统一 |
| read-only adversarial review | Yes, max 1 reviewer | 只输出 findings |

## 3. Worktree Need

当前修整阶段不需要 worktree，因为只写 docs。但未来 product code 或并行 spike 如触碰 executable files，必须用独立目录或 worktree。

## 4. Integrator Behavior

Integrator must:

1. own final architecture consistency;
2. reject duplicate/conflicting agent outputs;
3. merge only evidence-backed changes;
4. ensure DeepSeek/Qwen native invariants are not weakened;
5. verify TaskContract boundaries;
6. produce final report and go/no-go status.

## 5. Reviewer Behavior

Reviewer:

- read-only by default;
- reports findings, missing tests, contradictions;
- cannot modify ADR/Product Kernel/Security Model;
- must cite file/section;
- should focus on safety, model-scope drift, eval gaps.

## 6. Avoiding File Overwrites

Rules:

- Every parallel task gets explicit write paths.
- No two agents write the same file.
- Core docs can only be edited by Integrator.
- Spike outputs stay in isolated directories.
- If a shared interface change is required, stop parallel work.

## 7. Avoiding Duplicate Research

- Give each read-only agent a distinct question.
- Require source paths and confidence levels.
- Integrator deduplicates claims.
- Do not ask multiple agents to summarize the entire architecture.

## 8. Avoiding "Fast but Low Quality"

Quality gates:

- every spike has local test command;
- every parser/provider change has fixture;
- every native optimization change maps to eval;
- every security claim maps to threat/control;
- reviewer required before scaffold.

## 9. Final Aggregation

Final aggregation must include:

- changed files;
- conflicts fixed;
- model scope;
- DeepSeek/Qwen invariants preserved;
- provider layer;
- multi-agent policy;
- autonomy policy;
- Phase 0 order;
- go/no-go status;
- next 3 tasks.

## 10. Phase Strategy

| Stage | Max agents | Automatic execution | Human check | Go/No-Go |
|---|---:|---|---|---|
| Current convergence | 1 + reviewer | Yes within docs TaskContract | Final review | all convergence docs complete |
| Phase 0 | 4 | Yes for isolated docs/prototypes | Required for gate | 12 Phase 0 tasks pass |
| Spike stage | 4 | Yes in isolated dirs | Required before productization | spike tests pass |
| Scaffold stage | 1 | No until go/no-go | Required | all release blockers cleared |
| Implementation after interface freeze | 3 + reviewer | Contract-scoped | Required for core modules | tests/evals pass |

## 11. Codex Desktop Multi-Thread Recommendations

Current:

- Main thread: architecture convergence.
- Optional reviewer thread: read-only hardening review.
- No implementation threads.

Phase 0:

- Thread 1: schema/PlanApproval/ProviderConfig docs.
- Thread 2: DeepSeek/Qwen fixture specs.
- Thread 3: patch/command permission prototype specs.
- Thread 4: Research CSV profiler spike spec.
- Integrator thread merges reports only after outputs complete.

Implementation:

- Core runtime remains single-threaded.
- Independent implementation agents only after schemas freeze.
- Reviewer thread remains separate and read-only.

## 12. Codex Long-Task No-Review Usage

Allowed without step-by-step review:

- docs updates in allowed paths;
- fixture generation;
- isolated spike prototypes with test commands;
- read-only source research.

Not allowed without review:

- Product Kernel changes;
- Event schema changes;
- Permission/Patch/Model Router core;
- DeepSeek/Qwen native adapter strategy;
- security model;
- AGENTS.md core rules;
- scaffold/monorepo creation.

## 13. Phase 0 Arrangement

Phase 0 should start with:

1. Kernel schema consistency check.
2. PlanApproval vs Permission schema fix.
3. ProviderConfig/ModelAlias schema spike.
4. Native optimization consolidation check.

Then proceed to replay/parser/patch/permission/research spikes.

## 14. Spike Stage Arrangement

Each spike:

- owns one directory;
- has one test command;
- writes a limitations section;
- cannot be merged into product code without Integrator decision;
- must say whether it supports or contradicts architecture assumptions.

## 15. Implementation Stage Arrangement

Only after interface freeze:

- Rust event log/permission/patch modules can be implemented sequentially.
- GUI shell can prototype against mock event stream.
- Research Worker can prototype from manifest.
- Native adapters remain single-threaded strategy work, with parser fixtures parallelizable.

## 16. Go/No-Go Criteria

Go to Phase 0 spike:

- model scope converged;
- native invariants preserved;
- TaskContract defined;
- PlanApproval/Permission split defined;
- threat controls drafted.

Go to scaffold:

- Phase 0 12 tasks complete;
- security release blockers defined;
- event replay/patch/permission/parser/provider/research spikes complete;
- AGENTS draft accepted;
- no unresolved kernel/model-scope conflict.

## Execution Impact

- Current work remains single agent.
- Phase 0 can use up to 4 agents, isolated.
- Scaffold remains blocked until go/no-go checklist passes.

## Next Tasks

1. Execute Phase 0 task 01: kernel schema consistency and PlanApproval split.
2. Execute Phase 0 task 03: ProviderConfig/ModelAlias schema spike.
3. Execute Phase 0 task 09/10: DeepSeek/Qwen parser fixture spikes.

## Open Questions

- Should Codex Desktop reviewer outputs become formal ADR comments, or remain separate review reports?
- Should Phase 0 use worktrees or simple isolated directories since it is docs/prototypes only?
