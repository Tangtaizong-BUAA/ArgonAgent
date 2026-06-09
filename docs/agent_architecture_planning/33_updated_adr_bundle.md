# 33 Updated ADR Bundle

本文件解决的问题：把关键 ADR 更新为最新收敛状态。它不删除 `18_architecture_decision_records.md`，但本文件在模型范围、多智能体、bounded autonomy、PlanApproval/Permission 和 native optimization preservation 上优先。

修正旧文档的方式：将过早的 “Accepted” 降级为 Provisional Accepted，并新增缺失 ADR。

## ADR-001: Runtime API Shape

**Status:** Provisional Accepted.

### Context

Desktop GUI needs privileged local access, approvals, event streaming, and future CLI/remote adapters. Earlier ADR accepted both Tauri IPC and local HTTP/WebSocket as a combined target. Current convergence requires a safer staged approach.

### Options Considered

| Option | Pros | Cons |
|---|---|---|
| Tauri IPC only | Smallest desktop attack surface | CLI/remote later needs adapter |
| Local HTTP/WebSocket first | Easy CLI/streaming | Local API abuse risk |
| Tauri IPC first, HTTP/WebSocket after spike | Safer staged path | Requires later adapter validation |

### Decision

Tauri IPC is first. Local HTTP/WebSocket is a later adapter for CLI/remote runtime only after local API auth and streaming spike passes.

### Consequences

- GUI prototype can focus on desktop security.
- CLI is not blocked conceptually, but not first implementation driver.
- Runtime event envelope remains API-neutral.

### Risks

- IPC and HTTP APIs can drift later.
- CLI/TUI may require additional abstraction.

### Reversal Condition

Reverse if IPC cannot support required event streaming or local HTTP auth spike proves simpler and safe enough.

### Implementation Implication

Phase 0 must include an event-stream spike before productizing local HTTP/WebSocket.

## ADR-005: Model Provider Scope

**Status:** Provisional Accepted.

### Context

The product must be DeepSeek/Qwen native-first. Other models are useful but cannot dilute native optimization work.

### Options Considered

| Option | Pros | Cons |
|---|---|---|
| Full multi-model native optimization | Broad promise | Unbounded scope, weak DeepSeek/Qwen focus |
| DeepSeek/Qwen only with no others | Focused | No compatibility/baseline path |
| DeepSeek/Qwen native + CompatibleProvider layer | Focus and extensibility | Requires strict validation |

### Decision

DeepSeek and Qwen/Qwen3.6-27B are the only native optimized models. Claude/OpenAI/Codex/GPT/GLM/local/OpenAI-compatible/Anthropic-compatible/custom providers are compatible-only or baseline.

Existing DeepSeek/Qwen optimization is retained and must not be replaced by generic profiles.

### Consequences

- Native profiles can be deeply optimized.
- Compatible providers can be manually used or benchmarked without changing kernel assumptions.
- GUI must label native vs compatible clearly.

### Risks

- Users may expect Claude/OpenAI first-class support.
- Misconfiguration may make a compatible endpoint appear native.

### Reversal Condition

Reverse only if user explicitly starts a new native optimization program for another model family with dedicated docs, adapters, and eval gates.

### Implementation Implication

Implement `NativeModelProfile` only for DeepSeek/Qwen; implement `CompatibleProviderConfig` and `ModelAliasMapping` for others.

## ADR-006: Tech Stack

**Status:** Provisional Accepted for prototype choice.

### Context

Earlier docs recommended Tauri + React + Rust + Python sidecar. Current stage is not scaffold-ready; stack must be validated by spikes.

### Options Considered

| Option | Pros | Cons |
|---|---|---|
| Tauri + React + Rust + Python sidecar | Strong local safety and research ecosystem | Cross-language complexity |
| Electron + TypeScript + Python | Faster app iteration | Larger/security footprint |
| Python-first backend | Research-friendly | Weaker shell/file kernel boundary |

### Decision

Use Tauri + React + Rust runtime + Python sidecar as prototype choice, not final full implementation commitment.

### Consequences

Phase 0 must validate event log, patch validator, shell permission, and Tauri event stream before scaffold.

### Risks

Rust/Tauri may slow early iteration; Python packaging may be costly.

### Reversal Condition

Reverse if Phase 0 spikes show Rust/Tauri cannot meet event streaming, permission, patch, or packaging needs efficiently.

### Implementation Implication

No monorepo scaffold until Phase 0 go/no-go passes.

## ADR-009: Multi-Agent Orchestration

**Status:** Accepted policy.

### Context

Codex Desktop can use multiple threads/agents, but uncontrolled parallelism harms architecture coherence.

### Options Considered

| Option | Pros | Cons |
|---|---|---|
| Default multi-agent | Fast | High conflict/integration risk |
| Single agent only | Coherent | Underuses parallel research/spikes |
| Policy-driven multi-agent | Balanced | Needs explicit contracts |

### Decision

Multi-agent is policy-driven, not default. Kernel/security/native strategy/ADR/schema work is single-agent with optional reviewer. Parallelism is allowed for isolated research, fixtures, spikes, and frozen-interface modules.

### Consequences

- Default = Single Agent + Reviewer.
- Integrator owns merges.
- Worktree/isolated directories required for parallel product code/spikes.

### Risks

Policy may slow tasks that appear parallelizable.

### Reversal Condition

Reverse only if measured integration cost is consistently lower than expected and conflict rates stay low.

### Implementation Implication

Add `max_parallel_agents`, agent role, and write-scope rules to TaskContract.

## ADR-010: Bounded Autonomy

**Status:** Accepted policy.

### Context

Long tasks should not ask at every step, but unrestricted no-review execution is unsafe.

### Options Considered

| Option | Pros | Cons |
|---|---|---|
| Step-by-step approval | Safe | Too slow |
| Unrestricted autonomy | Fast | Unsafe/untraceable |
| TaskContract-bounded autonomy | Balanced | Requires upfront contract |

### Decision

Every long task must run under TaskContract with goal, scope, paths, tools, policies, retries, artifacts, stop conditions, reviewer/integrator requirements, and final report format.

### Consequences

Agents can act automatically inside bounds and must stop on violation.

### Risks

Contracts may be incomplete or too broad.

### Reversal Condition

Reverse only if TaskContract overhead blocks useful work and can be replaced by stronger runtime policy with equal auditability.

### Implementation Implication

TaskContract schema is Phase 0 before implementation.

## ADR-011: Plan Approval vs Permission

**Status:** Accepted correction.

### Context

`23_gui_user_flows.md` previously represented plan approval as `permission.requested` with `request_type = plan`. This mixes task governance and safety boundaries.

### Options Considered

| Option | Pros | Cons |
|---|---|---|
| Keep plan as permission | Reuses UI | Confuses safety audit |
| Separate PlanApproval and Permission | Clear semantics | More schemas/events |

### Decision

Plan approval is task governance. Permission is safety boundary. Use separate schemas/events:

- `PlanApprovalRequest`
- `PlanApprovalDecision`
- `PermissionRequest`
- `PermissionDecision`

They may share GUI drawer components but not event types.

### Consequences

Approved plans do not authorize shell/file/network/cloud/package/protected actions.

### Risks

UI may still blur the distinction if labels are weak.

### Reversal Condition

Do not reverse unless a formal policy model can prove plan governance and safety permission remain separable in one envelope.

### Implementation Implication

Update Product Kernel, GUI flows, schema tasks, and Phase 0 order.

## ADR-012: Native Optimization Preservation

**Status:** Accepted policy.

### Context

Existing docs already define DeepSeek/Qwen optimization in depth. Future edits must not collapse them into generic provider profiles.

### Options Considered

| Option | Pros | Cons |
|---|---|---|
| Rewrite native profiles fresh | Cleaner docs | Loses hard-earned specifics |
| Freeze all prior docs | Preserves detail | Blocks correction |
| Preserve invariants, allow eval-backed changes | Balanced | Requires discipline |

### Decision

DeepSeek/Qwen native optimizations are architecture assets. Changes must cite affected profile/rule, eval cases, promotion/rollback condition, and failure memory impact.

### Consequences

- DeepSeek reasoning/cache/parser policies remain intact.
- Qwen template/parser/thinking/context policies remain intact.
- Compatible providers cannot override native behavior.

### Risks

Some outdated assumptions may linger if not evaled.

### Reversal Condition

Only eval evidence or provider API changes can reverse a native invariant.

### Implementation Implication

Add native optimization consolidation check to Phase 0.

