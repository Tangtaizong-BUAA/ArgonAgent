# 29 Multi-Agent Orchestration Policy

本文件解决的问题：定义何时使用多智能体，何时禁止多智能体，以及 Integrator/Reviewer/Read-only/Implementation Agent 的边界。多智能体是 policy-driven，不是 default。

修正旧文档的方式：把“多 agent 并行”从产品愿景降级为受控执行策略，避免影响 Product Kernel、Event Schema、Security Model、DeepSeek/Qwen native adapter 等全局一致性模块。

## 1. Design Goal

**Decision:** Default = **Single Agent + Reviewer**.

多智能体只用于并行收益明显大于集成成本的任务，例如独立调研、独立 spike、eval fixture 生成、adversarial review、接口冻结后的独立模块实现。

**Rule:** Multi-agent is policy-driven, not default.

**Rationale:** 核心架构一致性比并行速度更重要。

**Implementation Impact:** TaskContract 必须声明 agent mode、max parallelism、write scopes、Integrator/Reviewer 要求。

**Eval Impact:** 多 agent 只影响 fixture/spike/implementation throughput，不得改变 DeepSeek/Qwen native eval promotion rules。

**Go/No-Go Impact:** 如果多 agent 可以并行修改 kernel/security/native adapter/ADR/schema，则 scaffold No-Go。

## 2. Risks

| Risk | Impact |
|---|---|
| 上下文不一致 | 架构决策冲突，重复劳动 |
| 接口冲突 | schema/API 不兼容 |
| 代码互相覆盖 | merge 成本高于并行收益 |
| 局部最优 | 单个 agent 完成局部实现但破坏整体架构 |
| 安全边界漂移 | Permission/Patch/Model Router core 出现不一致 |
| review 质量下降 | 快速并行产物无人整合 |

## 3. Default Single Agent Principle

**Rule:** 以下任务默认单 agent，由主 agent 直接完成，最多安排 reviewer 读报告：

- Product Kernel 设计；
- Event schema 设计；
- Database schema 设计；
- Permission Manager 核心逻辑；
- Patch Manager 核心逻辑；
- Model Router core；
- DeepSeek/Qwen native adapter 核心策略；
- Security model；
- ADR 决策；
- root architecture contract；
- AGENTS.md 核心规则。

## 4. Allowed Multi-Agent Cases

| Case | Allowed? | Conditions |
|---|---|---|
| 独立源码调研 | Yes | read-only, separate questions, no edits |
| 独立 spike prototype | Yes | separate directories, explicit acceptance tests |
| eval fixtures generation | Yes | non-overlapping fixture dirs |
| adversarial review | Yes | read-only findings only |
| 接口冻结后的独立模块实现 | Yes | disjoint files, integrator-owned merge |
| 多个互不重叠文档修订 | Yes | clear file ownership |

## 5. Banned Multi-Agent Cases

**Rule:** 明确禁止多 agent 并行修改或并行决策：

- Product Kernel；
- Event schema；
- Database schema；
- Permission Manager core；
- Patch Manager core；
- Model Router core；
- DeepSeek/Qwen native adapter core strategy；
- Security model；
- ADR；
- AGENTS.md core rules。

这些任务必须单线程完成，最多使用 reviewer 提意见。

## 6. Agent Roles

| Role | Can read | Can edit | Primary output | Notes |
|---|---|---|---|---|
| Integrator Agent | Yes | Yes, but only integration-owned files | final consolidated patch/report | Owns consistency and conflict resolution. |
| Reviewer Agent | Yes | No by default | findings, risk list, test gaps | Must not rewrite architecture decisions. |
| Read-only Agent | Yes | No | source analysis/report | Used for parallel research. |
| Implementation Agent | Yes | Yes, scoped paths only | bounded patch + tests | Only after interfaces freeze. |

## 7. Modes

### Single Agent Mode

| Field | Rule |
|---|---|
| Input | One TaskContract; full architecture context |
| Output | Single coherent artifact |
| Code edits | Allowed only by contract |
| Worktree | Not required |
| Max parallel | 1 + optional reviewer |
| Stop condition | scope violation, schema conflict, security uncertainty |
| Quality gate | self-check + reviewer if high-risk |

### Research Swarm Mode

| Field | Rule |
|---|---|
| Input | Independent research questions |
| Output | Reports with citations/paths |
| Code edits | No |
| Worktree | Not required |
| Max parallel | 4 read-only agents |
| Stop condition | overlapping research, conflicting claims without evidence |
| Quality gate | Integrator deduplicates and ranks confidence |

### Spike Parallel Mode

| Field | Rule |
|---|---|
| Input | Frozen spike contract and independent directories |
| Output | prototype + test command + limitations |
| Code edits | Yes, only under `spikes/` or `docs/prototypes/` |
| Worktree | Preferred if touching executable code |
| Max parallel | 4 |
| Stop condition | shared interface change required |
| Quality gate | spike-local test must pass; Integrator summarizes, not productizes |

### Implementation Shards Mode

| Field | Rule |
|---|---|
| Input | Frozen interfaces and disjoint write scopes |
| Output | module patch + tests |
| Code edits | Yes, scoped |
| Worktree | Required for product code |
| Max parallel | 3 implementation agents |
| Stop condition | interface change needed, overlapping files, failed contract tests |
| Quality gate | module tests + integration review |

### Adversarial Review Mode

| Field | Rule |
|---|---|
| Input | Existing docs/patches/eval results |
| Output | findings ordered by severity |
| Code edits | No |
| Worktree | Not required |
| Max parallel | 2 reviewers |
| Stop condition | reviewer starts rewriting rather than reviewing |
| Quality gate | findings must cite file/line or source section |

## 8. Multi-Agent Result Handling

### Aggregation

Integrator must:

1. collect outputs;
2. classify by claim, code change, fixture, risk;
3. deduplicate overlap;
4. mark conflicts;
5. select authoritative result;
6. produce final report with rejected alternatives.

### Deduplication

| Duplicate type | Resolution |
|---|---|
| Same source claim | Keep strongest evidence, cite both if useful. |
| Same fixture | Keep clearer fixture; merge edge cases only if non-overlapping. |
| Same code path | Stop and assign single owner. |
| Conflicting ADR advice | Main agent decides; reviewer can only recommend. |

### Conflict Resolution

1. If conflict touches kernel/security/model-native strategy, stop parallel work.
2. Integrator writes conflict summary.
3. Main agent decides or asks user.
4. Losing branch is not merged.

### Rollback

- Spike directories can be discarded.
- Worktree branches can be abandoned.
- Product files require PatchProposal and base hash validation.
- Event log records agent id and write scope.

## 9. Parallel Benefit Test

Use multi-agent only if all are true:

- tasks are independent;
- write scopes do not overlap;
- interface is frozen or no code edits;
- expected integration cost is lower than saved time;
- reviewer/integrator capacity exists;
- failure can be rolled back cleanly.

## 10. Anti-Patterns

- Parallel agents editing Product Kernel.
- Several agents drafting different database schemas.
- Implementation agents modifying shared types while others depend on them.
- Reviewer silently editing files.
- Using multi-agent because it is available, not because dependency graph supports it.
- Merging spikes directly into product code.

## 11. Recommended Parallelism by Project Stage

| Stage | Max agents | Allowed mode | Notes |
|---|---:|---|---|
| Current convergence | 1 + optional reviewer | Single Agent Mode | No parallel core docs edits. |
| Phase 0 hardening/spikes | 4 | Research Swarm / Spike Parallel | Separate dirs, tests required. |
| Early scaffold | 1 | Single Agent Mode | Project boundaries must be coherent. |
| Post-interface freeze | 3 + reviewer | Implementation Shards | Disjoint modules only. |
| Security/native adapter work | 1 + reviewer | Single Agent Mode | No parallel core modifications. |

## Execution Impact

- Add multi-agent mode field to TaskContract.
- Product Kernel and ADR work remain single-agent.
- Worktree isolation becomes mandatory for parallel product code.

## Next Tasks

1. Add `max_parallel_agents` and `agent_mode` to TaskContract schema.
2. Add reviewer-only task template.
3. Add spike directory ownership rules to AGENTS.md draft.

## Open Questions

- Should the product GUI expose multi-agent controls in v0, or keep them internal until worktree manager is mature?
- What is the maximum safe parallelism on typical local machines for Qwen3.6 local serving?
