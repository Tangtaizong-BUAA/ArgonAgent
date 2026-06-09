# 32 Architecture Consistency Fixes

本文件解决的问题：检查并记录本轮最终收敛中发现和修正的架构冲突。它是 consistency report，不是新愿景。

修正旧文档的方式：对 Product Kernel、GUI flows、ADR、Executive Summary、revised tasks、model optimization architecture 做了最小一致性更新，并新增模型范围、多智能体、bounded autonomy、Phase 0、AGENTS draft 等收敛文件。

## 1. Checks and Fixes

| # | Check | Result | Fix |
|---:|---|---|---|
| 1 | 是否仍把产品写成全模型优化平台 | Partial risk in older broad wording | Added `27`; patched `10` to use CompatibleProvider. |
| 2 | 是否仍把 Claude/OpenAI/GLM/local 写成 native | No current authoritative native claim found; risk in broad wording | `27` and `33` make them compatible-only. |
| 3 | 是否仍缺 compatible provider layer | Yes | Added `27_model_scope_and_provider_layer.md`. |
| 4 | 是否重复或弱化 DeepSeek 优化 | No weakening found; duplicated across docs | Added `28` invariants and source index. |
| 5 | 是否重复或弱化 Qwen 优化 | No weakening found; evidence weaker than DeepSeek | Added `28`; Phase 0 Qwen fixture/evidence tasks. |
| 6 | Executive Summary 是否仍建议直接 scaffold | Yes | Patched `14` to say No-Go for scaffold and next tasks are Phase 0. |
| 7 | Revised first tasks 是否与 Executive Summary 冲突 | Potential | Patched `24` with Phase 0 execution order note and schema scope updates. |
| 8 | Product Kernel 与 GUI flows 是否冲突 | Yes on plan approval | Patched `21` and `23`. |
| 9 | PermissionRequest 是否被错误用于 plan approval | Yes | Replaced GUI flow plan approval events with `plan.approval_*`; added PlanApproval types. |
| 10 | ADR 状态是否过早 accepted | Yes for ADR-001/005/006 | Patched `18`; added `33` with Provisional Accepted. |
| 11 | Multi-agent 是否被默认启用 | Not explicit enough | Added `29`: default Single Agent + Reviewer. |
| 12 | Long-task autonomy 是否没有边界 | Yes | Added `30`: TaskContract. |
| 13 | Phase 0 是否文档过多、spike 太靠后 | Yes | Added `docs/implementation/phase0_execution_order.md`. |
| 14 | Research Worker 是否被降级 | No; but needed phase gate | `22` preserved; Phase 0 Research CSV profiler added. |
| 15 | Compatible providers 是否污染 native optimization | Risk existed | `27`, `28`, `33`, `threat_control_matrix` block it. |
| 16 | AGENTS.md 草案是否反映最新约束 | Missing | Added `34_AGENTS_md_draft.md`. |

## 2. Specific Document Updates

### Product Kernel

Updated `21_product_kernel_v0.md`:

- Added `plan.approval_requested` / `plan.approval_decided` events.
- Added `POST /v0/plan_approvals/{id}/decision`.
- Added `PlanApprovalRequest` and `PlanApprovalDecision`.
- Clarified that `PermissionRequest` does not include plan approval.

**Decision:** Plan approval is task governance; permission is safety boundary.

### GUI Flows

Updated `23_gui_user_flows.md`:

- Replaced `permission.requested` with `request_type = plan` by `plan.approval_requested`.
- Replaced plan approval decision endpoint with `/v0/plan_approvals/{id}/decision`.
- Added governance vs permission note.

### ADR

Updated `18_architecture_decision_records.md` and added `33_updated_adr_bundle.md`:

- ADR-001 is Provisional Accepted; Tauri IPC first.
- ADR-005 is DeepSeek/Qwen native-first; compatible providers only for others.
- ADR-006 is prototype choice, not final implementation commitment.
- Added ADRs for multi-agent, bounded autonomy, PlanApproval vs Permission, native optimization preservation.

### Executive Summary

Updated `14_executive_summary.md`:

- Added compatible-only scope for non-native models.
- Changed stack language to prototype direction.
- Replaced scaffold next tasks with Phase 0 schema/provider/parser tasks.

### Revised Tasks

Updated `24_revised_first_30_codex_tasks.md`:

- Added note that `phase0_execution_order.md` is authoritative before scaffold.
- Expanded Task 01 to include PlanApproval, PermissionDecision, CompatibleProviderConfig, ModelAliasMapping.

### Model Optimization Architecture

Updated `10_model_optimization_architecture.md`:

- Added CompatibleProvider-only rule for Claude/OpenAI/GLM/local/custom.
- Clarified non-native systems cannot be native or override DeepSeek/Qwen policies.

### AGENTS.md Draft

Added `34_AGENTS_md_draft.md`:

- DeepSeek/Qwen-first product direction.
- Compatible provider rules.
- Multi-agent policy.
- TaskContract autonomy.
- File modification and testing rules.

## 3. Architecture Decisions

| Decision | Implementation Impact | Eval Impact | Go/No-Go Impact |
|---|---|---|---|
| DeepSeek/Qwen only native | Two native adapters; compatible layer for others | Native eval only for DeepSeek/Qwen | Scaffold blocked if compatible can be native |
| CompatibleProvider layer | ProviderConfig/ModelAlias schema required | Baseline eval only | Phase 0 provider spike required |
| PlanApproval separate from Permission | New schemas/events/endpoints | Governance decisions logged separately | Blocks scaffold until schema fixed |
| Multi-agent policy-driven | TaskContract max agents/write scopes | Multi-agent conflict tests later | No parallel kernel edits |
| Bounded autonomy | TaskContract required | Violation tests required | No unrestricted long tasks |

## 4. Open Questions

1. Should root `AGENTS.md` be replaced by the full draft or a shorter linked version?
2. Which Qwen3.6 deployment stack should be primary for first native eval?
3. Should local HTTP/WebSocket be postponed until after GUI IPC prototype, or spiked in Phase 0 as read-only event stream?
4. Should ProviderConfig schema live under `docs/schemas/provider/` or kernel schema directory?

## 5. Next Tasks

1. Execute Phase 0 Task 01/02: kernel schema consistency and PlanApproval/Permission schema split.
2. Execute Phase 0 Task 03: ProviderConfig/ModelAlias schema spike.
3. Execute Phase 0 Task 09/10: DeepSeek/Qwen parser fixture spikes.

