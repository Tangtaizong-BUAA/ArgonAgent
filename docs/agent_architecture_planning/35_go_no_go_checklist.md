# 35 Go/No-Go Checklist

本文件解决的问题：定义进入 scaffold / 正式实现前的检查清单。当前结论已再次更新：Phase 0 prototypes 已通过本地非破坏性检查，可以继续 runtime implementation；full prompt scaffold 不能默认启用，必须先通过 ContextBudgetManager 和 eval gate。

修正旧文档的方式：替代旧 Executive Summary 中“下一步 scaffold”的建议，将实现入口改为 Phase 0 go/no-go。

## Checklist

| # | Check | Current status | Evidence / required artifact | Go/No-Go impact |
|---:|---|---|---|---|
| 1 | 是否完成模型范围收敛 | Done | `27_model_scope_and_provider_layer.md` | Go for Phase 0 |
| 2 | 是否确认 DeepSeek/Qwen native-only | Done | `27`, `28`, `33` | Go for Phase 0 |
| 3 | 是否完成 ProviderConfig schema | Done | `docs/schemas/provider/compatible_provider_config.schema.json` | Go for scaffold |
| 4 | 是否完成 ModelAliasMapping schema | Done | `docs/schemas/provider/model_alias_mapping.schema.json` | Go for scaffold |
| 5 | 是否完成 Native optimization consolidation | Done | `28_native_optimization_consolidation.md` | Go for scaffold |
| 6 | 是否保留 DeepSeek 优化不变量 | Done as doc | `28` section 2 | Blocks if contradicted |
| 7 | 是否保留 Qwen 优化不变量 | Done as doc | `28` section 3 | Blocks if contradicted |
| 8 | 是否完成 TaskContract schema | Done | `docs/schemas/task_contract/task_contract.schema.json` | Go for scaffold |
| 9 | 是否修复 PlanApproval/Permission 不一致 | Done | `21`, `23`, `33`, `docs/schemas/kernel/plan_approval_*.schema.json` | Go for scaffold |
| 10 | 是否完成 Product Kernel consistency | Done for Phase 0 | schema validation passed | Go for scaffold |
| 11 | 是否完成 ADR 更新 | Done as doc | `33_updated_adr_bundle.md` | Go for Phase 0 |
| 12 | 是否完成 threat control matrix | Done as doc | `docs/security/threat_control_matrix.md` | Needs runnable controls |
| 13 | 是否完成 Phase 0 execution order | Done | `docs/implementation/phase0_execution_order.md` | Go for Phase 0 |
| 14 | 是否完成 AGENTS.md draft | Done | `34_AGENTS_md_draft.md` | User review needed |
| 15 | 是否完成 Event log replay spike | Done | `docs/prototypes/event_log_replay/`, `scripts/validate_event_sequence.py` | Go for scaffold |
| 16 | 是否完成 Patch validator spike | Done | `eval/fixtures/patch/`, `scripts/prototype_patch_validator.py` | Go for scaffold |
| 17 | 是否完成 Command permission classifier spike | Done | `eval/fixtures/shell/permission_cases.json`, `scripts/prototype_command_classifier.py` | Go for scaffold |
| 18 | 是否完成 DeepSeek parser fixture | Done | `eval/fixtures/deepseek/parser_golden.json` | Go for scaffold; native promotion still needs model eval later |
| 19 | 是否完成 Qwen parser/executor fixture | Done | `eval/fixtures/qwen/parser_golden.json` | Go for scaffold; native promotion still needs model eval later |
| 20 | 是否完成 Compatible provider adapter spike | Done | provider schemas + `docs/prototypes/provider_adapter/README.md` | Go for scaffold |
| 21 | 是否完成 Research CSV profiler spike | Done | `eval/fixtures/research/csv-quality-small/`, `scripts/prototype_csv_profiler.py` | Go for scaffold |
| 22 | 是否有 release-blocking security rules | Done with tests mapped | `threat_control_matrix.md` + Phase 0 scripts | Go for scaffold |
| 23 | 是否可以开始 monorepo scaffold | Already minimally created | Phase 0 checks passed | Continue runtime; no dependency install |
| 24 | 如果不能，阻塞项是什么 | None for minimal scaffold | Native promotion/model eval still future | Does not block runtime |
| 25 | 下一步最优任务是什么 | Context-budget-aware runtime wiring | See below | Start scaffold-budget work |
| 26 | 是否完成 context budget/scaffold policy | Done as doc | `36_context_budget_and_scaffold_policy.md` | Go for budgeted runtime |
| 27 | 是否完成 ContextBudgetManager v0 | Done in runtime | `crates/runtime/src/context_budget.rs` | Go for prompt assembler wiring |
| 28 | 是否记录 DeepSeek S3 / Qwen S1/S2 / Compatible S0 | Done | `36` + CLI `context-budget-smoke` | Blocks full scaffold if absent |
| 29 | 是否承认 Qwen full ClaudeCode prompt 风险 | Done | `36` Qwen policy | Blocks Qwen full scaffold default |
| 30 | 是否定义 scaffold eval gates | v0 done | `scripts/run_scaffold_eval.py`, `scripts/run_scaffold_comparison_eval.py`, `36` Eval Gates | Blocks full prompt promotion only for future live/offline quality gates |

## Current Decision

**Decision:** Phase 0 prototype gate passed. Minimal scaffold exists. **Go for runtime continuation and context-budget-aware prompt/context wiring.**

**No-Go:** Do not promote full prompt scaffold as quality default until live/offline quality evals exist. `ContextBudget` is consumed by prompt assembly and deterministic scaffold comparison gates now exist.

## Remaining Non-Blocking Risks

1. Native DeepSeek/Qwen promotion still requires live/offline model evals.
2. Local HTTP/WebSocket remains deferred until auth/streaming spike.
3. Full prompt scaffold needs explicit eval before default promotion.
4. Scaffold must not install dependencies or run package managers.

## Next Optimal Tasks

1. **Record scaffold-level budget telemetry in native model-call event logs.**
2. **Add budget-aware context retrieval before prompt assembly.**
3. **Add live/offline quality evals for Qwen lite vs full and DeepSeek full vs lite.**

## Go Criteria for Scaffold

Scaffold only after:

- all P0 release-blocking controls have schema/fixture/prototype coverage;
- DeepSeek/Qwen native invariants remain intact;
- compatible providers cannot be marked native;
- TaskContract violation behavior is specified and testable;
- Product Kernel event flow replays one full coding task;
- Patch validator blocks stale/protected/ambiguous writes;
- command classifier blocks destructive/injection commands;
- AGENTS.md policy is accepted by user.

## Open Questions

- Should schema tasks produce JSON Schema only, or also TypeScript/Rust type sketches?
- Should Phase 0 scripts live under `scripts/` or `spikes/phase0/`?
- Should AGENTS.md draft be shortened before root adoption?
